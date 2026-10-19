//! SafeInCloud's legacy two-layer encrypted database and lossless XML editing.
//!
//! Format interoperability is based on Max Schmitt's MIT-licensed
//! golang-safe-in-cloud. See THIRD_PARTY_LICENSE. CBC provides no authenticity.
use aes::{Aes128, Aes192, Aes256};
use anyhow::{anyhow, bail, ensure, Context, Result};
use cbc::cipher::{
    block_padding::{NoPadding, Pkcs7},
    BlockDecryptMut, BlockEncryptMut, KeyIvInit,
};
use flate2::{write::ZlibEncoder, Compression, Decompress, FlushDecompress, Status};
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use rand::{rngs::OsRng, Rng, RngCore};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use xml::reader::{EventReader, XmlEvent};
use xmltree::{Element, EmitterConfig, ParserConfig, XMLNode};
use zeroize::{Zeroize, Zeroizing};

pub const MAX_FILE: usize = 32 * 1024 * 1024;
pub const MAX_XML: usize = 64 * 1024 * 1024;

#[derive(Clone)]
pub struct Field {
    pub name: String,
    pub kind: String,
    pub value: String,
}
#[derive(Clone)]
pub struct Card {
    pub id: String,
    pub title: String,
    pub fields: Vec<Field>,
    pub notes: String,
    pub favorite: bool,
    pub label_ids: Vec<String>,
}
#[derive(Clone)]
pub struct Label {
    pub id: String,
    pub name: String,
}

pub struct Vault {
    pub path: PathBuf,
    pub dirty: bool,
    pub last_backup: Option<PathBuf>,
    password: Zeroizing<String>,
    version: u8,
    document: Vec<XMLNode>,
    root_index: usize,
    snapshot: [u8; 32],
}

fn password_bytes(password: &str, version: u8) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(if version == 1 {
        password.encode_utf16().map(|unit| unit as u8).collect()
    } else {
        password.as_bytes().to_vec()
    })
}

fn array<'a>(data: &'a [u8], position: &mut usize) -> Result<&'a [u8]> {
    let length = *data
        .get(*position)
        .ok_or_else(|| anyhow!("Truncated database header"))? as usize;
    *position += 1;
    let end = position
        .checked_add(length)
        .ok_or_else(|| anyhow!("Invalid header"))?;
    let value = data
        .get(*position..end)
        .ok_or_else(|| anyhow!("Truncated database header"))?;
    *position = end;
    Ok(value)
}

fn pack(output: &mut Vec<u8>, value: &[u8]) -> Result<()> {
    ensure!(value.len() <= 255, "Invalid header array");
    output.push(value.len() as u8);
    output.extend_from_slice(value);
    Ok(())
}

fn decrypt_cbc(key: &[u8], iv: &[u8], data: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    ensure!(
        iv.len() == 16 && !data.is_empty() && data.len().is_multiple_of(16),
        "Incorrect password or damaged database"
    );
    let result = match key.len() {
        16 => cbc::Decryptor::<Aes128>::new_from_slices(key, iv)
            .map_err(|_| anyhow!("Invalid cipher key"))?
            .decrypt_padded_vec_mut::<NoPadding>(data),
        24 => cbc::Decryptor::<Aes192>::new_from_slices(key, iv)
            .map_err(|_| anyhow!("Invalid cipher key"))?
            .decrypt_padded_vec_mut::<NoPadding>(data),
        32 => cbc::Decryptor::<Aes256>::new_from_slices(key, iv)
            .map_err(|_| anyhow!("Invalid cipher key"))?
            .decrypt_padded_vec_mut::<NoPadding>(data),
        _ => bail!("Invalid payload key"),
    }
    .map_err(|_| anyhow!("Incorrect password or damaged database"))?;
    Ok(Zeroizing::new(result))
}

fn encrypt_cbc(key: &[u8], iv: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    Ok(cbc::Encryptor::<Aes256>::new_from_slices(key, iv)
        .map_err(|_| anyhow!("Invalid cipher key"))?
        .encrypt_padded_vec_mut::<Pkcs7>(data))
}

// HMAC's verification uses a constant-time comparison. Neither operand is
// exposed through an ordinary string or byte equality check.
fn verify_key(expected: &[u8], actual: &[u8]) -> Result<()> {
    ensure!(
        actual.len() == expected.len(),
        "Incorrect password or damaged key descriptor"
    );
    let mut a = <Hmac<Sha256> as Mac>::new_from_slice(b"SafeInCloud key verification")?;
    a.update(expected);
    let mut b = <Hmac<Sha256> as Mac>::new_from_slice(b"SafeInCloud key verification")?;
    b.update(actual);
    a.verify_slice(&b.finalize().into_bytes())
        .map_err(|_| anyhow!("Incorrect password or damaged key descriptor"))
}

/// Reject DTDs and bound nesting before xmltree's recursive tree construction.
fn check_xml_limits(xml: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(xml).context("Database XML must be UTF-8")?;
    ensure!(xml.len() <= MAX_XML, "XML exceeds 64 MiB limit");
    let mut rest = text;
    let mut depth = 0usize;
    let mut elements = 0usize;
    while let Some(start) = rest.find('<') {
        rest = &rest[start..];
        let special = if rest.starts_with("<!--") {
            Some(("-->", 4))
        } else if rest.starts_with("<![CDATA[") {
            Some(("]]>", 9))
        } else if rest.starts_with("<?") {
            Some(("?>", 2))
        } else {
            None
        };
        if let Some((end, offset)) = special {
            let stop = rest[offset..]
                .find(end)
                .ok_or_else(|| anyhow!("Invalid XML"))?
                + offset
                + end.len();
            rest = &rest[stop..];
            continue;
        }
        ensure!(
            !rest.starts_with("<!"),
            "DTD and entity declarations are forbidden"
        );
        let mut quote = None;
        let mut stop = None;
        for (i, ch) in rest.bytes().enumerate().skip(1) {
            if let Some(q) = quote {
                if ch == q {
                    quote = None;
                }
            } else if ch == b'\'' || ch == b'"' {
                quote = Some(ch);
            } else if ch == b'>' {
                stop = Some(i);
                break;
            }
        }
        let stop = stop.ok_or_else(|| anyhow!("Invalid XML"))?;
        if rest.starts_with("</") {
            depth = depth
                .checked_sub(1)
                .ok_or_else(|| anyhow!("Invalid XML nesting"))?;
        } else {
            elements += 1;
            ensure!(elements <= 250_000, "Too many XML elements");
            if !rest[..stop].trim_end().ends_with('/') {
                depth += 1;
            }
            ensure!(depth <= 128, "XML nesting exceeds limit");
        }
        rest = &rest[stop + 1..];
    }
    ensure!(depth == 0, "Invalid XML nesting");
    Ok(())
}

fn parse_document(xml: &[u8]) -> Result<(Vec<XMLNode>, usize)> {
    check_xml_limits(xml)?;
    let config = ParserConfig::new()
        .ignore_comments(false)
        .trim_whitespace(false)
        .whitespace_to_characters(true)
        .cdata_to_characters(false);
    // xmltree's built-in parser strips attribute prefixes. Build its tree from
    // events ourselves so future namespaced attributes remain intact.
    let mut document = Vec::new();
    let mut stack: Vec<Element> = Vec::new();
    for event in EventReader::new_with_config(xml, config) {
        let node = match event.context("Invalid database XML")? {
            XmlEvent::StartElement {
                name,
                attributes,
                namespace,
            } => {
                let mut element = Element::new(&name.local_name);
                element.prefix = name.prefix;
                element.namespace = name.namespace;
                element.namespaces = Some(namespace);
                for attribute in attributes {
                    let name = attribute
                        .name
                        .prefix
                        .map(|p| format!("{p}:{}", attribute.name.local_name))
                        .unwrap_or(attribute.name.local_name);
                    element.attributes.insert(name, attribute.value);
                }
                stack.push(element);
                continue;
            }
            XmlEvent::EndElement { .. } => {
                XMLNode::Element(stack.pop().ok_or_else(|| anyhow!("Invalid XML nesting"))?)
            }
            XmlEvent::Characters(s) | XmlEvent::Whitespace(s) => XMLNode::Text(s),
            XmlEvent::CData(s) => XMLNode::CData(s),
            XmlEvent::Comment(s) => XMLNode::Comment(s),
            XmlEvent::ProcessingInstruction { name, data } => {
                XMLNode::ProcessingInstruction(name, data)
            }
            XmlEvent::StartDocument { .. } | XmlEvent::EndDocument => continue,
        };
        if let Some(parent) = stack.last_mut() {
            parent.children.push(node);
        } else {
            document.push(node);
        }
    }
    let roots: Vec<usize> = document
        .iter()
        .enumerate()
        .filter_map(|(i, n)| n.as_element().map(|_| i))
        .collect();
    ensure!(roots.len() == 1, "Expected one XML document element");
    let index = roots[0];
    let root = document[index].as_element().unwrap();
    ensure!(
        root.name == "database" && root.namespace.is_none(),
        "Unexpected XML document"
    );
    Ok((document, index))
}

pub fn decrypt(data: &[u8], password: &str) -> Result<Vec<u8>> {
    ensure!(data.len() <= MAX_FILE, "Database exceeds 32 MiB limit");
    ensure!(
        data.len() >= 3 && data[..2] == [5, 5] && matches!(data[2], 1 | 2),
        "Unsupported database format/version"
    );
    let mut position = 3;
    let salt = array(data, &mut position)?;
    let iv = array(data, &mut position)?;
    ensure!(!salt.is_empty() && iv.len() == 16, "Invalid outer header");
    let key_salt = array(data, &mut position)?;
    let wrapped = array(data, &mut position)?;
    let mut outer = Zeroizing::new([0u8; 32]);
    pbkdf2_hmac::<Sha1>(
        &password_bytes(password, data[2]),
        salt,
        10_000,
        &mut *outer,
    );
    let descriptor = decrypt_cbc(&*outer, iv, wrapped)?;
    let mut inner_pos = 0;
    let payload_iv = array(&descriptor, &mut inner_pos)?;
    let payload_key = array(&descriptor, &mut inner_pos)?;
    let check = array(&descriptor, &mut inner_pos)?;
    if !check.is_empty() {
        let mut expected = Zeroizing::new([0u8; 32]);
        pbkdf2_hmac::<Sha1>(payload_key, key_salt, 1_000, &mut *expected);
        verify_key(&*expected, check)?;
    }
    let compressed = decrypt_cbc(payload_key, payload_iv, &data[position..])?;
    let mut decoder = Decompress::new(true);
    let mut xml = Zeroizing::new(Vec::new());
    let mut chunk = Zeroizing::new([0u8; 65_536]);
    loop {
        let previous_in = decoder.total_in();
        let previous_out = decoder.total_out();
        let status = decoder
            .decompress(
                &compressed[previous_in as usize..],
                &mut *chunk,
                FlushDecompress::None,
            )
            .context("Invalid compressed database")?;
        let produced = (decoder.total_out() - previous_out) as usize;
        ensure!(xml.len() + produced <= MAX_XML, "XML exceeds 64 MiB limit");
        xml.extend_from_slice(&chunk[..produced]);
        if status == Status::StreamEnd {
            break;
        }
        ensure!(
            decoder.total_in() > previous_in || produced > 0,
            "Truncated compressed database"
        );
    }
    let (mut document, _) = parse_document(&xml)?;
    wipe_document(&mut document);
    Ok(std::mem::take(&mut *xml))
}

pub fn encrypt(xml: &[u8], password: &str, version: u8) -> Result<Vec<u8>> {
    ensure!(
        matches!(version, 1 | 2) && xml.len() <= MAX_XML,
        "Unsupported version or oversized XML"
    );
    let mut salt = [0; 64];
    let mut iv = [0; 16];
    let mut key_salt = [0; 64];
    let mut key = Zeroizing::new([0; 32]);
    let mut payload_iv = [0; 16];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut iv);
    OsRng.fill_bytes(&mut key_salt);
    OsRng.fill_bytes(&mut *key);
    OsRng.fill_bytes(&mut payload_iv);
    let mut outer = Zeroizing::new([0; 32]);
    let mut check = Zeroizing::new([0; 32]);
    pbkdf2_hmac::<Sha1>(
        &password_bytes(password, version),
        &salt,
        10_000,
        &mut *outer,
    );
    pbkdf2_hmac::<Sha1>(&*key, &key_salt, 1_000, &mut *check);
    let mut descriptor = Zeroizing::new(Vec::new());
    pack(&mut descriptor, &payload_iv)?;
    pack(&mut descriptor, &*key)?;
    pack(&mut descriptor, &*check)?;
    let mut compressed = Zeroizing::new(Vec::new());
    let mut compressor = ZlibEncoder::new(&mut *compressed, Compression::default());
    compressor.write_all(xml)?;
    compressor.finish()?;
    let mut output = vec![5, 5, version];
    pack(&mut output, &salt)?;
    pack(&mut output, &iv)?;
    pack(&mut output, &key_salt)?;
    pack(&mut output, &encrypt_cbc(&*outer, &iv, &descriptor)?)?;
    output.extend(encrypt_cbc(&*key, &payload_iv, &compressed)?);
    ensure!(
        output.len() <= MAX_FILE,
        "Encrypted database exceeds 32 MiB limit"
    );
    Ok(output)
}

fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    File::open(path)
        .with_context(|| format!("Cannot open {}", path.display()))?
        .take((MAX_FILE + 1) as u64)
        .read_to_end(&mut data)?;
    ensure!(data.len() <= MAX_FILE, "Database exceeds 32 MiB limit");
    Ok(data)
}
fn digest(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}
fn attr(element: &Element, key: &str) -> String {
    element.attributes.get(key).cloned().unwrap_or_default()
}
fn flag(element: &Element, key: &str) -> bool {
    element
        .attributes
        .get(key)
        .is_some_and(|s| s.eq_ignore_ascii_case("true"))
}
fn text(element: &Element) -> String {
    element
        .children
        .iter()
        .take_while(|node| !matches!(node, XMLNode::Element(_)))
        .filter_map(|node| match node {
            XMLNode::Text(s) | XMLNode::CData(s) => Some(s.as_str()),
            _ => None,
        })
        .collect()
}
fn known(element: &Element, name: &str) -> bool {
    element.name == name && element.namespace.is_none() && element.prefix.is_none()
}
fn children<'a>(element: &'a Element, name: &'a str) -> impl Iterator<Item = &'a Element> {
    element
        .children
        .iter()
        .filter_map(XMLNode::as_element)
        .filter(move |e| known(e, name))
}

fn set_text(element: &mut Element, value: &str) {
    // Replace the element's leading text, retaining nested unknown nodes and tails.
    let end = element
        .children
        .iter()
        .position(|node| matches!(node, XMLNode::Element(_)))
        .unwrap_or(element.children.len());
    let mut insertion = None;
    let mut index = 0;
    while index < end.min(element.children.len()) {
        if matches!(
            element.children[index],
            XMLNode::Text(_) | XMLNode::CData(_)
        ) {
            if insertion.is_none() {
                insertion = Some(index);
            }
            let mut removed = element.children.remove(index);
            wipe_node(&mut removed);
        } else {
            index += 1;
        }
        // Stop when removal reaches the first nested element.
        if element
            .children
            .get(index)
            .is_some_and(|n| matches!(n, XMLNode::Element(_)))
        {
            break;
        }
    }
    element
        .children
        .insert(insertion.unwrap_or(0), XMLNode::Text(value.to_owned()));
}

fn stamp(element: &mut Element) {
    let old = element
        .attributes
        .get("time_stamp")
        .and_then(|s| s.parse::<u128>().ok())
        .unwrap_or(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let next = now.max(old.saturating_add(2));
    if element
        .attributes
        .get("first_stamp")
        .is_none_or(String::is_empty)
    {
        let first = element
            .attributes
            .get("prev_stamp")
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| {
                if old == 0 {
                    next.to_string()
                } else {
                    old.to_string()
                }
            });
        element.attributes.insert("first_stamp".into(), first);
    }
    element
        .attributes
        .insert("time_stamp".into(), next.to_string());
}

impl Vault {
    pub fn open(path: impl AsRef<Path>, password: &str) -> Result<Self> {
        Self::from_encrypted(path.as_ref(), &read_bounded(path.as_ref())?, password)
    }
    pub fn from_encrypted(path: impl AsRef<Path>, data: &[u8], password: &str) -> Result<Self> {
        let xml = Zeroizing::new(decrypt(data, password)?);
        let (document, root_index) = parse_document(&xml)?;
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            dirty: false,
            last_backup: None,
            password: Zeroizing::new(password.to_owned()),
            version: data[2],
            document,
            root_index,
            snapshot: digest(data),
        })
    }
    fn root(&self) -> &Element {
        self.document[self.root_index].as_element().unwrap()
    }
    fn root_mut(&mut self) -> &mut Element {
        self.document[self.root_index].as_mut_element().unwrap()
    }
    fn card_mut(&mut self, id: &str) -> Result<&mut Element> {
        self.root_mut()
            .children
            .iter_mut()
            .filter_map(XMLNode::as_mut_element)
            .find(|e| known(e, "card") && e.attributes.get("id").is_some_and(|value| value == id))
            .ok_or_else(|| anyhow!("Card no longer exists"))
    }
    pub fn cards(&self) -> Vec<Card> {
        children(self.root(), "card")
            .filter(|e| !flag(e, "template") && !flag(e, "deleted"))
            .map(|e| Card {
                id: attr(e, "id"),
                title: attr(e, "title"),
                favorite: flag(e, "star"),
                fields: children(e, "field")
                    .map(|f| Field {
                        name: attr(f, "name"),
                        kind: attr(f, "type"),
                        value: text(f),
                    })
                    .collect(),
                notes: children(e, "notes").next().map(text).unwrap_or_default(),
                label_ids: children(e, "label_id").map(text).collect(),
            })
            .collect()
    }
    pub fn labels(&self) -> Vec<Label> {
        children(self.root(), "label")
            .filter(|e| !flag(e, "deleted"))
            .map(|e| Label {
                id: attr(e, "id"),
                name: attr(e, "name"),
            })
            .collect()
    }
    pub fn upsert(
        &mut self,
        id: Option<&str>,
        title: &str,
        fields: &[Field],
        notes: &str,
    ) -> Result<String> {
        let ident = if let Some(id) = id {
            id.to_owned()
        } else {
            let ident = loop {
                let candidate = OsRng.gen_range(1..2_147_483_647u32).to_string();
                if !self
                    .root()
                    .children
                    .iter()
                    .filter_map(XMLNode::as_element)
                    .any(|e| e.attributes.get("id") == Some(&candidate))
                {
                    break candidate;
                }
            };
            let mut card = Element::new("card");
            card.attributes.insert("id".into(), ident.clone());
            card.attributes.insert("symbol".into(), "key".into());
            card.attributes.insert("color".into(), "gray".into());
            self.root_mut().children.push(XMLNode::Element(card));
            ident
        };
        let card = self.card_mut(&ident)?;
        card.attributes.insert("title".into(), title.into());
        let insertion = card
            .children
            .iter()
            .position(|n| n.as_element().is_some_and(|e| known(e, "field")))
            .unwrap_or(0);
        let mut existing: Vec<Element> = Vec::new();
        let mut retained = Vec::new();
        for node in std::mem::take(&mut card.children) {
            match node {
                XMLNode::Element(e) if known(&e, "field") => existing.push(e),
                other => retained.push(other),
            }
        }
        let mut replacements = Vec::new();
        for field in fields {
            let mut node = if let Some(i) = existing
                .iter()
                .position(|e| e.attributes.get("name") == Some(&field.name))
            {
                existing.remove(i)
            } else if !existing.is_empty() {
                existing.remove(0)
            } else {
                Element::new("field")
            };
            node.attributes.insert("name".into(), field.name.clone());
            node.attributes.insert("type".into(), field.kind.clone());
            set_text(&mut node, &field.value);
            replacements.push(XMLNode::Element(node));
        }
        for mut discarded in existing {
            wipe_element(&mut discarded);
        }
        retained.splice(insertion..insertion, replacements);
        card.children = retained;
        if children(card, "notes").next().is_none() {
            card.children.push(XMLNode::Element(Element::new("notes")));
        }
        let note = card
            .children
            .iter_mut()
            .filter_map(XMLNode::as_mut_element)
            .find(|e| known(e, "notes"))
            .unwrap();
        set_text(note, notes);
        stamp(card);
        self.dirty = true;
        Ok(ident)
    }
    pub fn set_favorite(&mut self, id: &str, favorite: bool) -> Result<()> {
        let card = self.card_mut(id)?;
        card.attributes.insert("star".into(), favorite.to_string());
        stamp(card);
        self.dirty = true;
        Ok(())
    }
    pub fn delete(&mut self, id: &str) -> Result<()> {
        let card = self.card_mut(id)?;
        card.attributes.insert("deleted".into(), "true".into());
        stamp(card);
        self.dirty = true;
        Ok(())
    }
    pub fn encrypted(&self) -> Result<Vec<u8>> {
        let mut xml = Zeroizing::new(Vec::new());
        // Serialize document-level comments/PIs as well as nodes within database.
        for node in &self.document {
            match node {
                XMLNode::Element(e) => e.write_with_config(
                    &mut *xml,
                    EmitterConfig::new()
                        .write_document_declaration(false)
                        .autopad_comments(false),
                )?,
                XMLNode::Comment(s) => write!(&mut *xml, "<!--{}-->", s)?,
                XMLNode::ProcessingInstruction(name, data) => {
                    write!(&mut *xml, "<?{}", name)?;
                    if let Some(s) = data {
                        write!(&mut *xml, " {}", s)?;
                    }
                    write!(&mut *xml, "?>")?;
                }
                XMLNode::Text(s) => xml.extend_from_slice(s.as_bytes()),
                XMLNode::CData(s) => write!(&mut *xml, "<![CDATA[{}]]>", s)?,
            }
        }
        encrypt(&xml, &self.password, self.version)
    }
    pub fn snapshot_digest(&self) -> [u8; 32] {
        self.snapshot
    }
    /// Restore an encrypted pending-edit recovery image's original disk baseline.
    pub fn restore_pending_baseline(&mut self, snapshot: [u8; 32]) {
        self.snapshot = snapshot;
        self.dirty = true;
    }
    /// Call only after exactly these encrypted bytes have been persisted locally.
    pub fn mark_saved(&mut self, encrypted: &[u8]) {
        self.snapshot = digest(encrypted);
        self.dirty = false;
    }
    pub fn replace_from_encrypted(&mut self, data: &[u8]) -> Result<()> {
        ensure!(
            !self.dirty,
            "Unsaved changes; save or discard before reloading"
        );
        let fresh = Self::from_encrypted(&self.path, data, &self.password)?;
        *self = fresh;
        Ok(())
    }
    pub fn validate_encrypted(&self, data: &[u8]) -> Result<()> {
        let _xml = Zeroizing::new(decrypt(data, &self.password)?);
        Ok(())
    }
    pub fn reload_from_disk(&mut self) -> Result<()> {
        self.replace_from_encrypted(&read_bounded(&self.path)?)
    }
    pub fn save(&mut self) -> Result<()> {
        let original = read_bounded(&self.path)?;
        ensure!(
            digest(&original) == self.snapshot,
            "Database changed outside this app; reload before saving"
        );
        if !self.dirty {
            return Ok(());
        }
        let encrypted = self.encrypted()?;
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let filename = self
            .path
            .file_name()
            .ok_or_else(|| anyhow!("Database has no filename"))?
            .to_string_lossy();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let backup = parent.join(format!(
            "{filename}.backup-{nanos}-{:016x}",
            OsRng.next_u64()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut backup_file = options
            .open(&backup)
            .context("Cannot create encrypted backup")?;
        backup_file.write_all(&original)?;
        backup_file.sync_all()?;
        self.last_backup = Some(backup);
        let mut temporary = tempfile::Builder::new()
            .prefix(&format!(".{filename}-"))
            .tempfile_in(parent)?;
        temporary.write_all(&encrypted)?;
        temporary.as_file().sync_all()?;
        ensure!(
            digest(&read_bounded(&self.path)?) == self.snapshot,
            "Database changed while saving; original preserved"
        );
        temporary
            .persist(&self.path)
            .map_err(|e| anyhow!("Cannot atomically replace database: {}", e.error))?;
        File::open(parent)?.sync_all()?;
        self.mark_saved(&encrypted);
        Ok(())
    }
}

fn wipe_element(element: &mut Element) {
    for value in element.attributes.values_mut() {
        value.zeroize();
    }
    for child in &mut element.children {
        wipe_node(child);
    }
}
fn wipe_node(node: &mut XMLNode) {
    match node {
        XMLNode::Element(e) => wipe_element(e),
        XMLNode::Text(s) | XMLNode::CData(s) | XMLNode::Comment(s) => s.zeroize(),
        XMLNode::ProcessingInstruction(s, data) => {
            s.zeroize();
            if let Some(data) = data {
                data.zeroize();
            }
        }
    }
}
fn wipe_document(document: &mut [XMLNode]) {
    for node in document {
        wipe_node(node);
    }
}
impl Drop for Vault {
    fn drop(&mut self) {
        wipe_document(&mut self.document);
    }
}

#[cfg(test)]
#[path = "vault_tests.rs"]
mod tests;
