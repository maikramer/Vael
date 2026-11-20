use super::*;

const SCENE: &str = r#"// Praça
#"Pórtico"
jackdaw_scene_types::node_id::SceneNodeId(18446744073709551615)
bevy_world_xml::bsn::WorldNode { tag: "Group", attrs: [], text: "" }
Children [
    #piso demo::Transform { translation: glam::Vec3 { x: -3.5 // rebaixado
        , y: 0.0 }, label: "rio -- lua" }
    --
    #piso other::Light
]
--
#sky other::Light
"#;

fn scene() -> String {
    SCENE.to_owned()
}

fn set(node: &str, field: &str, value: &str) -> Edit {
    Edit {
        node: node.into(),
        attr: None,
        field: Some(field.into()),
        value: value.into(),
    }
}

#[test]
fn selects_physical_roots_children_and_full_precision_ids() {
    let input = scene();
    let nodes = index(&input).unwrap();
    assert_eq!(
        nodes.iter().map(|n| n.path.as_str()).collect::<Vec<_>>(),
        ["0", "0/0", "0/1", "1"]
    );
    let root = select(&nodes, "id:18446744073709551615").unwrap();
    assert_eq!(
        (root.name.as_deref(), root.tag.as_deref(), root.line),
        (Some("Pórtico"), Some("Group"), 2)
    );
    assert!(select(&nodes, "name:piso").is_err());
    assert_eq!(
        &input[select(&nodes, "1").unwrap().range.clone()],
        "#sky other::Light"
    );
}

#[test]
fn native_field_replaces_only_its_literal_and_keeps_comments() {
    let original = scene();
    let mut text = original.clone();
    apply(&mut text, &set("0/0", "Transform.translation.x", "-0.25")).unwrap();
    assert_eq!(text, original.replace("-3.5", "-0.25"));
    apply(
        &mut text,
        &set("0/0", "demo::Transform.label", r#""água \"azul\"""#),
    )
    .unwrap();
    assert_eq!(
        text,
        original
            .replace("-3.5", "-0.25")
            .replace(r#""rio -- lua""#, r#""água \"azul\"""#)
    );
    jackdaw_bsn::parse_bsn_text(&text).unwrap();
    assert!(apply(&mut text, &set("0/0", "Transform.label", "true, false")).is_err());
}

#[test]
fn attr_insert_then_update_round_trips_unicode_and_escaped_delimiters() {
    let mut text = scene().replace('\n', "\r\n");
    let mut edit = Edit {
        node: "0".into(),
        attr: Some("sinal".into()),
        field: None,
        value: "ponte".into(),
    };
    apply(&mut text, &edit).unwrap();
    let first = text.clone();
    edit.value = "á\\b \"--\"".into();
    apply(&mut text, &edit).unwrap();
    assert_eq!(text, first.replace(r#""ponte""#, r#""á\\b \"--\"""#));
    assert!(!text.replace("\r\n", "").contains('\n'));
    jackdaw_bsn::parse_bsn_text(&text).unwrap();
}

#[test]
fn insert_appends_one_sibling_for_leaf_empty_and_terminated_lists() {
    for (original, expected) in [
        ("#p", "0/0"),
        ("#p Children []", "0/0"),
        ("#p Children [#a]", "0/1"),
        ("#p Children [#a -- // fim\n]", "0/1"),
    ] {
        let mut text = original.to_owned();
        let nodes = index(&text).unwrap();
        let (range, block) = document::insert_child(&text, &nodes[0], "#b Children [#c]").unwrap();
        text.replace_range(range, &block);
        jackdaw_bsn::parse_bsn_text(&text).unwrap();
        let nodes = index(&text).unwrap();
        assert_eq!(select(&nodes, "name:b").unwrap().path, expected);
        assert_eq!(
            select(&nodes, "name:c").unwrap().path,
            format!("{expected}/0")
        );
        assert_eq!(text.matches("Children").count(), 2);
    }
}

#[test]
fn failed_patch_or_duplicate_insert_keeps_the_original() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("scene.bsn");
    let edits = dir.path().join("batch.json");
    fs::write(&file, scene()).unwrap();
    fs::write(&edits, r#"[{"node":"0","attr":"key","value":"first"},{"node":"99","attr":"key","value":"second"}]"#).unwrap();
    let target = || Target {
        file: file.clone(),
        dry_run: false,
    };
    let error = Command::Patch {
        target: target(),
        edits,
    }
    .run()
    .unwrap_err();
    assert!(error.to_string().contains("edição 2"));
    let fragment = dir.path().join("fragment.bsn");
    fs::write(
        &fragment,
        "#b jackdaw_scene_types::node_id::SceneNodeId(18446744073709551615)",
    )
    .unwrap();
    assert!(
        Command::Insert {
            target: target(),
            parent: "0".into(),
            fragment
        }
        .run()
        .is_err()
    );
    assert_eq!(fs::read_to_string(file).unwrap(), scene());
}

#[test]
fn native_binary_roundtrip_preserves_preamble_values_and_existing_destinations() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("source.bsn");
    let packed = dir.path().join("copy.bsb");
    let decoded = dir.path().join("copy.bsn");
    let source = "// jackdaw 0.20.2 | bevy 0.20\n#night demo::Sky { time: -1.0 }";
    fs::write(&original, source).unwrap();
    transcode(&original, &packed).unwrap();
    assert!(
        fs::read(&packed)
            .unwrap()
            .starts_with(&jackdaw_bsn::binary::MAGIC)
    );
    let text = read_scene(&packed).unwrap();
    assert!(text.starts_with("// jackdaw 0.20.2 | bevy 0.20\n"));
    let nodes = index(&text).unwrap();
    let field = document::field(&text, select(&nodes, "name:night").unwrap(), "Sky.time").unwrap();
    assert_eq!(text[field].parse::<f64>().unwrap(), -1.0);
    assert!(transcode(&packed, &original).is_err());
    assert_eq!(fs::read_to_string(&original).unwrap(), source);
    transcode(&packed, &decoded).unwrap();
    assert_eq!(fs::read_to_string(decoded).unwrap(), text);
}

#[test]
fn template_fields_keep_asset_references_and_report_ambiguous_short_types() {
    let source = r#"#a :"base.bsn" @"extra.bsn" @demo::Sky { time: 0.0 } other::Sky { time: 9.0 }"#;
    let mut text = source.to_owned();
    assert!(apply(&mut text, &set("0", "Sky.time", "2.5")).is_err());
    apply(&mut text, &set("0", "demo::Sky.time", "2.5")).unwrap();
    assert_eq!(text, source.replace("time: 0.0", "time: 2.5"));
    jackdaw_bsn::parse_bsn_text(&text).unwrap();
}
