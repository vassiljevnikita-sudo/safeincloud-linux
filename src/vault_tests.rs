use super::*;
use std::fs;

#[test]
fn reads_unicode_envelopes_written_by_the_predecessor_implementation() {
    let expected = include_bytes!("../tests/fixtures/interop.xml");
    for data in [
        include_bytes!("../tests/fixtures/python-v1-unicode.bin").as_slice(),
        include_bytes!("../tests/fixtures/python-v2-unicode.bin").as_slice(),
    ] {
        assert_eq!(
            decrypt(data, "päss🔒-Interop").unwrap().as_slice(),
            expected
        );
    }
}

const XML: &[u8] = br#"<?before preserve?><database future="yes" xmlns:x="urn:future"><!--keep--><label id="9" name="Home"/><card id="7" title="Old" time_stamp="123" future="x" star="true"><field name="Secret" type="password" unknown="yes" x:unknown="also"><![CDATA[fake]]><extra foo="bar"/></field><notes>old</notes><label_id>9</label_id><image opaque="yes"/></card><unknown a="b"/></database><!--after-->"#;
const PASSWORD: &str = "päss🔒";

fn field(value: &str) -> Field {
    Field {
        name: "Secret".into(),
        kind: "password".into(),
        value: value.into(),
    }
}
fn fixture() -> &'static [u8] {
    include_bytes!("../tests/fixtures/upstream-demo.db")
}

#[test]
fn public_upstream_fixture_and_wrong_password() {
    let mut vault = Vault::from_encrypted("unused.db", fixture(), "foobar").unwrap();
    assert!(children(vault.root(), "card")
        .any(|c| c.attributes.get("id").is_some_and(|id| id == "101")));
    assert!(vault.labels().iter().any(|l| l.name == "Geschäftlich"));
    let count = vault.cards().len();
    assert!(count > 0);
    let visible_id = vault.cards()[0].id.clone();
    vault.set_favorite(&visible_id, true).unwrap();
    let saved = vault.encrypted().unwrap();
    assert_eq!(
        Vault::from_encrypted("unused.db", &saved, "foobar")
            .unwrap()
            .cards()
            .len(),
        count
    );
    assert!(decrypt(fixture(), "wrong").is_err());
}

#[test]
fn legacy_unicode_bytes_match_utf16_code_units() {
    assert_eq!(&*password_bytes(PASSWORD, 1), &[112, 228, 115, 115, 61, 18]);
    assert_eq!(&*password_bytes(PASSWORD, 2), PASSWORD.as_bytes());
    for version in [1, 2] {
        let encrypted = encrypt(XML, PASSWORD, version).unwrap();
        assert_eq!(decrypt(&encrypted, PASSWORD).unwrap(), XML);
        let vault = Vault::from_encrypted("unused", &encrypted, PASSWORD).unwrap();
        let saved = vault.encrypted().unwrap();
        assert_eq!(saved[2], version);
        assert_ne!(saved, vault.encrypted().unwrap());
    }
}

#[test]
fn editing_preserves_unknown_xml_and_membership() {
    let original = encrypt(XML, PASSWORD, 2).unwrap();
    let mut vault = Vault::from_encrypted("unused", &original, PASSWORD).unwrap();
    vault
        .upsert(Some("7"), "New", &[field("new-secret")], "new notes")
        .unwrap();
    let encrypted = vault.encrypted().unwrap();
    let xml = String::from_utf8(decrypt(&encrypted, PASSWORD).unwrap()).unwrap();
    for fragment in [
        "<!--keep-->",
        "<?before preserve?>",
        "<!--after-->",
        "future=\"yes\"",
        "future=\"x\"",
        "unknown=\"yes\"",
        "x:unknown=\"also\"",
        "<extra",
        "<image",
        "<unknown",
        "first_stamp=\"123\"",
    ] {
        assert!(
            xml.contains(fragment),
            "Missing preserved XML fragment: {fragment}"
        );
    }
    let reopened = Vault::from_encrypted("unused", &encrypted, PASSWORD).unwrap();
    let card = &reopened.cards()[0];
    assert_eq!(card.title, "New");
    assert_eq!(card.fields[0].value, "new-secret");
    assert_eq!(card.notes, "new notes");
    assert_eq!(card.label_ids, ["9"]);
    assert!(card.favorite);
    let root = reopened.root();
    let card = root.get_child("card").unwrap();
    assert!(attr(card, "time_stamp").parse::<u128>().unwrap() > 123);
}

#[test]
fn namespaced_extensions_cannot_collide_with_known_elements() {
    let xml = br#"<database xmlns:x="urn:extension"><x:card id="7" title="Extension"/><card id="7" title="Original"><x:field name="Secret" x:data="opaque">extension field</x:field><field name="Secret" type="password">original</field><x:notes>extension note</x:notes><notes>original note</notes></card><x:label id="9" name="Extension"/></database>"#;
    let mut vault =
        Vault::from_encrypted("unused", &encrypt(xml, PASSWORD, 2).unwrap(), PASSWORD).unwrap();
    assert_eq!(vault.cards().len(), 1);
    assert!(vault.labels().is_empty());
    vault
        .upsert(Some("7"), "Edited", &[field("edited value")], "edited note")
        .unwrap();
    let encrypted = vault.encrypted().unwrap();
    let reopened = Vault::from_encrypted("unused", &encrypted, PASSWORD).unwrap();
    assert_eq!(reopened.cards()[0].fields.len(), 1);
    assert_eq!(reopened.cards()[0].fields[0].value, "edited value");
    assert_eq!(reopened.cards()[0].notes, "edited note");
    let saved_xml = String::from_utf8(decrypt(&encrypted, PASSWORD).unwrap()).unwrap();
    assert!(saved_xml.contains("extension field"));
    assert!(saved_xml.contains("extension note"));
    assert!(saved_xml.contains("title=\"Extension\""));
    assert!(saved_xml.contains("x:data=\"opaque\""));
}

#[test]
fn saves_are_atomic_encrypted_and_backed_up() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("demo.db");
    let original = encrypt(XML, PASSWORD, 2).unwrap();
    fs::write(&path, &original).unwrap();
    let mut vault = Vault::open(&path, PASSWORD).unwrap();
    vault
        .upsert(None, "Another", &[field("new-secret")], "")
        .unwrap();
    vault.save().unwrap();
    assert!(!vault.dirty);
    assert_eq!(
        fs::read(vault.last_backup.as_ref().unwrap()).unwrap(),
        original
    );
    assert_eq!(vault.snapshot_digest(), digest(&fs::read(&path).unwrap()));
    assert_eq!(Vault::open(&path, PASSWORD).unwrap().cards().len(), 2);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    vault.save().unwrap(); // A clean save must not generate another backup.
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn conflicting_edits_are_not_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("demo.db");
    fs::write(&path, encrypt(XML, PASSWORD, 2).unwrap()).unwrap();
    let mut vault = Vault::open(&path, PASSWORD).unwrap();
    vault.delete("7").unwrap();
    fs::write(&path, b"external change").unwrap();
    assert!(vault
        .save()
        .unwrap_err()
        .to_string()
        .contains("changed outside"));
    assert_eq!(fs::read(&path).unwrap(), b"external change");
    assert!(vault.dirty);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn trash_is_recoverable_and_favorite_retains_fields() {
    let encrypted = encrypt(XML, PASSWORD, 2).unwrap();
    let mut vault = Vault::from_encrypted("unused", &encrypted, PASSWORD).unwrap();
    vault.set_favorite("7", false).unwrap();
    assert!(!vault.cards()[0].favorite);
    assert_eq!(vault.cards()[0].fields[0].value, "fake");
    vault.delete("7").unwrap();
    assert!(vault.cards().is_empty());
    let card = vault.root().get_child("card").unwrap();
    assert!(flag(card, "deleted"));
    assert!(card.get_child("field").is_some());
}

#[test]
fn replacement_refuses_pending_edits_and_recovery_preserves_baseline() {
    let original = encrypt(XML, PASSWORD, 2).unwrap();
    let fresh = encrypt(
        b"<database><card id=\"4\" title=\"Cloud\"/></database>",
        PASSWORD,
        1,
    )
    .unwrap();
    let mut vault = Vault::from_encrypted("same.db", &original, PASSWORD).unwrap();
    vault.validate_encrypted(&fresh).unwrap();
    assert_eq!(vault.cards()[0].title, "Old");
    vault.replace_from_encrypted(&fresh).unwrap();
    assert_eq!(vault.path, PathBuf::from("same.db"));
    assert_eq!(vault.cards()[0].title, "Cloud");
    assert_eq!(vault.snapshot_digest(), digest(&fresh));
    assert!(!vault.dirty);
    vault.delete("4").unwrap();
    assert!(vault.replace_from_encrypted(&original).is_err());
    let recovery = vault.encrypted().unwrap();
    let mut recovered = Vault::from_encrypted("same.db", &recovery, PASSWORD).unwrap();
    recovered.restore_pending_baseline(digest(&fresh));
    assert!(recovered.dirty);
    assert_eq!(recovered.snapshot_digest(), digest(&fresh));
}

#[test]
fn rejects_truncation_versions_dtds_and_deep_xml() {
    for length in (0..150).chain(std::iter::once(fixture().len() - 1)) {
        assert!(
            decrypt(&fixture()[..length], "foobar").is_err(),
            "Accepted prefix {length}"
        );
    }
    let mut unknown = fixture().to_vec();
    unknown[2] = 99;
    assert!(decrypt(&unknown, "foobar").is_err());
    for xml in [
        b"<wrong/>".as_slice(),
        b"<!DOCTYPE database [<!ENTITY x 'bad'>]><database>&x;</database>",
    ] {
        assert!(decrypt(&encrypt(xml, "test", 2).unwrap(), "test").is_err());
    }
    let deep = format!(
        "<database>{}{}</database>",
        "<nested>".repeat(129),
        "</nested>".repeat(129)
    );
    assert!(decrypt(&encrypt(deep.as_bytes(), "test", 2).unwrap(), "test").is_err());
}

#[test]
fn decompression_is_bounded() {
    let oversized = vec![b' '; MAX_XML + 1];
    assert!(encrypt(&oversized, "test", 2).is_err());
    // Construct a syntactically valid envelope whose expansion crosses the limit.
    let mut compressor = ZlibEncoder::new(Vec::new(), Compression::fast());
    compressor.write_all(&oversized).unwrap();
    let compressed = compressor.finish().unwrap();
    let salt = [b's'; 16];
    let iv = [b'i'; 16];
    let key = [b'k'; 32];
    let payload_iv = [b'j'; 16];
    let mut outer = [0; 32];
    pbkdf2_hmac::<Sha1>(b"test", &salt, 10_000, &mut outer);
    let mut check = [0; 32];
    pbkdf2_hmac::<Sha1>(&key, &salt, 1_000, &mut check);
    let mut descriptor = Vec::new();
    pack(&mut descriptor, &payload_iv).unwrap();
    pack(&mut descriptor, &key).unwrap();
    pack(&mut descriptor, &check).unwrap();
    let mut envelope = vec![5, 5, 2];
    for value in [
        &salt[..],
        &iv[..],
        &salt[..],
        &encrypt_cbc(&outer, &iv, &descriptor).unwrap()[..],
    ] {
        pack(&mut envelope, value).unwrap();
    }
    envelope.extend(encrypt_cbc(&key, &payload_iv, &compressed).unwrap());
    assert!(decrypt(&envelope, "test")
        .unwrap_err()
        .to_string()
        .contains("64 MiB"));
}

#[test]
fn descriptor_verification_and_zlib_end_marker_are_required() {
    let salt = [b's'; 16];
    let iv = [b'i'; 16];
    let key = [b'k'; 32];
    let payload_iv = [b'j'; 16];
    let mut outer = [0; 32];
    pbkdf2_hmac::<Sha1>(b"test", &salt, 10_000, &mut outer);
    let mut good_check = [0; 32];
    pbkdf2_hmac::<Sha1>(&key, &salt, 1_000, &mut good_check);
    let mut compressor = ZlibEncoder::new(Vec::new(), Compression::default());
    compressor.write_all(b"<database/>").unwrap();
    let compressed = compressor.finish().unwrap();
    for (check, body) in [
        ([0; 32], &compressed[..]),
        (good_check, &compressed[..compressed.len() - 2]),
    ] {
        let mut descriptor = Vec::new();
        pack(&mut descriptor, &payload_iv).unwrap();
        pack(&mut descriptor, &key).unwrap();
        pack(&mut descriptor, &check).unwrap();
        let mut envelope = vec![5, 5, 2];
        for value in [
            &salt[..],
            &iv[..],
            &salt[..],
            &encrypt_cbc(&outer, &iv, &descriptor).unwrap()[..],
        ] {
            pack(&mut envelope, value).unwrap();
        }
        envelope.extend(encrypt_cbc(&key, &payload_iv, body).unwrap());
        assert!(decrypt(&envelope, "test").is_err());
    }
}
