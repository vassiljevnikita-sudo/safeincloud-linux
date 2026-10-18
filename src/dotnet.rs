//! Read only the OAuth client initialization in a user-supplied managed assembly.
//! The executable is parsed as data and is never executed.
use anyhow::{bail, Result};
use dotnetdll::{
    prelude::*,
    resolved::{
        il::Instruction,
        members::{MethodReferenceParent, MethodSource, UserMethod},
        types::{BaseType, MethodType, TypeSource},
    },
};
use std::{fs::File, io::Read, path::Path};
use zeroize::Zeroizing;

pub struct ClientSecrets {
    pub client_id: Zeroizing<String>,
    pub client_secret: Zeroizing<String>,
}

pub fn extract_client(path: &Path) -> Result<ClientSecrets> {
    extract(path).map_err(|_| {
        anyhow::anyhow!("Original desktop client configuration unavailable or unsupported.")
    })
}

fn extract(path: &Path) -> Result<ClientSecrets> {
    let file = File::open(path)?;
    if file.metadata()?.len() > 256 * 1024 * 1024 {
        bail!("assembly size");
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(256 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 256 * 1024 * 1024 {
        bail!("assembly size");
    }
    let res = Resolution::parse(
        &bytes,
        ReadOptions {
            lazy_method_bodies: true,
            lazy_attributes: true,
            ..Default::default()
        },
    )?;
    let types: Vec<_> = res
        .enumerate_type_definitions()
        .filter(|(_, t)| {
            t.name == "GDriveDriver" && t.namespace.as_deref() == Some("SafeInCloud.Clouds.GDrive")
        })
        .collect();
    if types.len() != 1 {
        bail!("driver type");
    }
    let methods: Vec<_> = res
        .enumerate_methods(types[0].0)
        .filter(|(_, m)| m.name == "GetAuthTask")
        .collect();
    if methods.len() != 1 {
        bail!("driver method");
    }
    let body = res.method_body(methods[0].0)?;
    let (mut id, mut secret) = (None, None);
    let mut pending: Option<Zeroizing<String>> = None;
    for instruction in &body.instructions {
        match instruction {
            Instruction::LoadString(text) => {
                pending = Some(Zeroizing::new(String::from_utf16(text)?))
            }
            Instruction::NoOperation => {}
            Instruction::CallVirtual {
                param0: MethodSource::User(UserMethod::Reference(index)),
                ..
            } => {
                let reference = &res[*index];
                let owner = match &reference.parent {
                    MethodReferenceParent::Type(MethodType::Base(base)) => match base.as_ref() {
                        BaseType::Type {
                            source: TypeSource::User(user),
                            ..
                        } => user.type_name(&res),
                        _ => String::new(),
                    },
                    _ => String::new(),
                };
                if owner.rsplit('.').next() == Some("ClientSecrets") {
                    let slot = match reference.name.as_ref() {
                        "set_ClientId" => Some(&mut id),
                        "set_ClientSecret" => Some(&mut secret),
                        _ => None,
                    };
                    if let Some(slot) = slot {
                        if slot.is_some() {
                            bail!("ambiguous initialization");
                        }
                        *slot = Some(
                            pending
                                .take()
                                .ok_or_else(|| anyhow::anyhow!("missing literal"))?,
                        );
                    }
                }
                pending = None;
            }
            _ => pending = None,
        }
    }
    let client_id = id.ok_or_else(|| anyhow::anyhow!("missing id"))?;
    let client_secret = secret.ok_or_else(|| anyhow::anyhow!("missing secret"))?;
    if !client_id.ends_with(".apps.googleusercontent.com")
        || client_id.len() > 1024
        || client_secret.is_empty()
        || client_secret.len() > 4096
    {
        bail!("invalid client");
    }
    Ok(ClientSecrets {
        client_id,
        client_secret,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn synthetic_assembly() -> Vec<u8> {
        let mut res = Resolution::new(Module::new("synthetic.dll"));
        res.assembly = Some(Assembly::new("synthetic"));
        let external =
            res.push_assembly_reference(ExternalAssemblyReference::new("Synthetic.Google"));
        let secrets = res
            .push_type_reference(type_ref! { Google.Apis.Auth.OAuth2.ClientSecrets in #external });
        let owner: MethodType = BaseType::class(secrets).into();
        let id = res.push_method_reference(method_ref! { void @owner::set_ClientId(string) });
        let secret =
            res.push_method_reference(method_ref! { void @owner::set_ClientSecret(string) });
        let driver = res.push_type_definition(TypeDefinition::new(
            Some("SafeInCloud.Clouds.GDrive".into()),
            "GDriveDriver",
        ));
        res.push_method(
            driver,
            Method::new(
                Accessibility::Public,
                msig! { static void () },
                "GetAuthTask",
                Some(dotnetdll::resolved::body::Method::new(asm! {
                    load_string "synthetic.apps.googleusercontent.com";
                    call_virtual id;
                    load_string "synthetic-client-secret";
                    call_virtual secret;
                    Return;
                })),
            ),
        );
        res.write(WriteOptions::default()).unwrap()
    }
    #[test]
    fn extracts_only_the_named_driver_method_setters() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), synthetic_assembly()).unwrap();
        let secrets = extract_client(temp.path()).unwrap();
        assert_eq!(
            secrets.client_id.as_str(),
            "synthetic.apps.googleusercontent.com"
        );
        assert_eq!(secrets.client_secret.as_str(), "synthetic-client-secret");
    }
    #[test]
    fn malformed_assembly_error_is_sanitized() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), b"synthetic-private-credential").unwrap();
        let error = extract_client(temp.path()).err().unwrap().to_string();
        assert!(!error.contains("synthetic-private"));
        assert!(!error.contains(&temp.path().display().to_string()));
    }
}
