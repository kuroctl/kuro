//! Gabarits sans logique : références, filtres, erreurs.

use std::collections::BTreeMap;

use kuro_prefab::template::{Context, Template, TemplateError};
use serde_json::json;

fn render(source: &str, context: &Context<'_>) -> Result<String, TemplateError> {
    Template::parse(source)?.render(context)
}

#[test]
fn settings_ports_resources_and_server_are_rendered() {
    let settings = BTreeMap::from([
        ("motd".to_owned(), json!("Hello")),
        ("slots".to_owned(), json!(20)),
        ("pvp".to_owned(), json!(false)),
    ]);
    let context = Context {
        settings: Some(&settings),
        ports: BTreeMap::from([("game".to_owned(), (Some(25570), 25565))]),
        memory_mb: 2048,
        cpu_percent: 150,
        server_id: "s-1".to_owned(),
        server_name: "Survie".to_owned(),
        server_web_url: Some("https://abc123defg.web.kuroctl.test".to_owned()),
        content: BTreeMap::new(),
        content_values: BTreeMap::new(),
    };

    let rendered = render(
        "{{ settings.motd }}/{{settings.slots}}/{{ settings.pvp }} {{ ports.game.host }}->{{ ports.game.container }} \
         {{ resources.memory_mb }}M {{ resources.cpu_percent }}% {{ server.id }} {{ server.name }} {{ server.web_url }}",
        &context,
    )
    .unwrap();

    assert_eq!(
        rendered,
        "Hello/20/false 25570->25565 2048M 150% s-1 Survie https://abc123defg.web.kuroctl.test"
    );
}

#[test]
fn a_server_without_site_has_no_web_url() {
    // Prefab web sans domaine des sites, ou serveur de jeu : la référence est sans valeur, et
    // `default` reste possible pour un prefab qui sait s'en passer.
    let context = Context::default();

    let error = render("{{ server.web_url }}", &context).unwrap_err();
    assert_eq!(error, TemplateError::Missing("server.web_url".to_owned()));
    assert_eq!(
        render(
            "{{ server.web_url | default(\"http://localhost\") }}",
            &context
        )
        .unwrap(),
        "http://localhost"
    );
}

#[test]
fn filters_apply_in_order() {
    let settings = BTreeMap::from([
        ("loader".to_owned(), json!("fabric")),
        ("empty".to_owned(), json!("")),
    ]);
    let context = Context {
        settings: Some(&settings),
        content: BTreeMap::from([("mods".to_owned(), vec!["12".to_owned(), "34".to_owned()])]),
        ..Context::default()
    };

    assert_eq!(
        render("{{ settings.loader | upper }}", &context).unwrap(),
        "FABRIC"
    );
    assert_eq!(
        render("{{ settings.empty | default(\"none\") | upper }}", &context).unwrap(),
        "NONE"
    );
    assert_eq!(
        render("-mods={{ content.mods.ids | join(\",\") }}", &context).unwrap(),
        "-mods=12,34"
    );
    assert_eq!(
        render("{{ content.mods.ids | join(\"|\") }}", &context).unwrap(),
        "12|34"
    );
}

#[test]
fn a_list_must_be_joined() {
    let context = Context::default();

    assert_eq!(
        render("{{ content.mods.ids }}", &context),
        Err(TemplateError::ListWithoutJoin(
            "content.mods.ids".to_owned()
        ))
    );
}

#[test]
fn an_unset_optional_setting_renders_empty() {
    let context = Context::default();

    assert_eq!(
        render("seed={{ settings.level_seed }}", &context).unwrap(),
        "seed="
    );
    assert_eq!(
        render("{{ settings.level_seed | default(\"x\") }}", &context).unwrap(),
        "x"
    );
}

#[test]
fn an_unknown_port_is_an_error() {
    let context = Context::default();

    assert_eq!(
        render("{{ ports.game.host }}", &context),
        Err(TemplateError::Missing("ports.game.host".to_owned()))
    );
}

#[test]
fn nothing_but_the_known_language_is_accepted() {
    for (source, error) in [
        ("{{ settings.motd", TemplateError::Unclosed),
        (
            "{{ env.HOME }}",
            TemplateError::InvalidReference("env.HOME".to_owned()),
        ),
        (
            "{{ settings.a.b }}",
            TemplateError::InvalidReference("settings.a.b".to_owned()),
        ),
        (
            "{{ settings.motd | exec(\"id\") }}",
            TemplateError::UnknownFilter("exec".to_owned()),
        ),
        (
            "{{ settings.motd | join }}",
            TemplateError::InvalidArgument("join".to_owned()),
        ),
    ] {
        assert_eq!(Template::parse(source), Err(error), "{source}");
    }
}

#[test]
fn text_without_expressions_is_kept_verbatim() {
    let context = Context::default();

    assert_eq!(
        render("plain } { text", &context).unwrap(),
        "plain } { text"
    );
}

#[test]
fn values_of_a_content_field_are_prefixed_then_joined() {
    let context = Context {
        content_values: BTreeMap::from([(
            "mods.mod_ids".to_owned(),
            vec!["Hydrocraft".to_owned(), "BetterSorting".to_owned()],
        )]),
        ..Context::default()
    };

    assert_eq!(
        render(
            "{{ content.mods.values.mod_ids | prefix(\"\\\") | join(\";\") }}",
            &context
        )
        .unwrap(),
        r"\Hydrocraft;\BetterSorting"
    );
    // Aucune valeur : rien, pas un préfixe seul.
    assert_eq!(
        render(
            "{{ content.other.values.mod_ids | prefix(\"\\\") | join(\";\") }}",
            &context
        )
        .unwrap(),
        ""
    );
}
