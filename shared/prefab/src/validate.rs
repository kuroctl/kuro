//! Validation d'un prefab : règles du JSON Schema que le typage ne porte pas (motifs, bornes),
//! puis contrôles croisés qu'un schéma ne peut pas exprimer (références de gabarits, chemins
//! sous un volume, valeurs par défaut, traductions).
//!
//! Le JSON Schema publié (`prefabs/schema/prefab.v1.schema.json`) et cette validation sont
//! comparés par les tests sur les prefabs officiels et sur des cas piégés.

use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::manifest::{
    ContentInstallMode, Field, FieldType, FileFormat, FileMode, MAX_ACTION_TIMEOUT_SECS, Manifest,
    Protocol, Ready, SCHEMA_V1, Service, Step,
};
use crate::package::Package;
use crate::settings;
use crate::template::{Reference, Template, is_key, is_slug};

/// Problème trouvé, avec son emplacement dans le manifest (`services.web.image`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub path: String,
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

static IMAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z0-9.-]+(:[0-9]+)?/[a-z0-9._/-]+(:[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}|@sha256:[a-f0-9]{64})$")
        .unwrap_or_else(|_| unreachable!("static regex"))
});
static USER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[1-9][0-9]*:[0-9]+$").unwrap_or_else(|_| unreachable!("static regex"))
});
/// Service d'un enregistrement SRV (`_minecraft._tcp`).
static SRV_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^_[a-z0-9][a-z0-9-]{0,61}\._(tcp|udp)$")
        .unwrap_or_else(|_| unreachable!("static regex"))
});
static ENV_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").unwrap_or_else(|_| unreachable!("static regex"))
});
static SHA256: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-f0-9]{64}$").unwrap_or_else(|_| unreachable!("static regex"))
});

/// Formats dont Kuro sait réécrire des clés (`managed-keys`).
pub const MANAGED_KEY_FORMATS: &[FileFormat] = &[
    FileFormat::Properties,
    FileFormat::Ini,
    FileFormat::Json,
    FileFormat::Yaml,
];

/// `{{ … }}` seul, du début à la fin de l'argument (espaces compris) : un paramètre ne se
/// concatène jamais à du texte ou à une autre référence.
fn is_whole_expression(arg: &str) -> bool {
    let trimmed = arg.trim();
    trimmed.starts_with("{{") && trimmed.ends_with("}}") && trimmed.matches("{{").count() == 1
}

/// Formats où une clé est un chemin (`a.b.c`) et une valeur garde son type (ADR-0039).
const STRUCTURED_KEY_FORMATS: &[FileFormat] = &[FileFormat::Json, FileFormat::Yaml];

/// Valeur d'une clé gérée : scalaire, ou liste de scalaires.
fn is_managed_value(value: &Value) -> bool {
    match value {
        Value::String(_) | Value::Number(_) | Value::Bool(_) => true,
        Value::Array(items) => items
            .iter()
            .all(|item| matches!(item, Value::String(_) | Value::Number(_) | Value::Bool(_))),
        Value::Null | Value::Object(_) => false,
    }
}

pub fn validate(package: &Package) -> Vec<Issue> {
    let mut check = Checker::default();
    let manifest = &package.manifest;
    check.identity(manifest);
    for (name, service) in &manifest.services {
        check.service(manifest, name, service);
    }
    check.resources(manifest);
    check.settings(manifest);
    check.files(package);
    check.install(package);
    check.backup(manifest);
    check.content(manifest);
    check.lifecycle(manifest);
    check.web_ports(manifest);
    check.actions(manifest);
    check.templates(manifest);
    check.migrations(manifest);
    check.locales(package);
    check.issues
}

#[derive(Default)]
struct Checker {
    issues: Vec<Issue>,
}

impl Checker {
    fn fail(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.issues.push(Issue {
            path: path.into(),
            message: message.into(),
        });
    }

    fn identity(&mut self, m: &Manifest) {
        if m.schema != SCHEMA_V1 {
            self.fail("schema", format!("expected `{SCHEMA_V1}`"));
        }
        if !is_slug(&m.id) {
            self.fail("id", "expected lowercase letters, digits and dashes");
        }
        let mut seen = HashSet::new();
        for tag in &m.tags {
            if !is_slug(tag) || !seen.insert(tag) {
                self.fail("tags", format!("invalid or duplicate tag `{tag}`"));
            }
        }
        if let Some(requirement) = &m.requires.kuro
            && semver::VersionReq::parse(requirement).is_err()
        {
            self.fail("requires.kuro", "invalid semver requirement");
        }
        if m.steam.is_some_and(|steam| steam.app == 0) {
            self.fail("steam.app", "expected a Steam app id");
        }
        for (name, need) in m.requires.resources.iter() {
            let at = format!("requires.resources.{name}");
            // Bornes acceptées par l'API pour un serveur.
            let range = match name {
                "memory_mb" => 64..=1_048_576,
                "cpu_percent" => 1..=12_800,
                _ => 1..=104_857_600,
            };
            for (bound, value) in [("min", need.min), ("recommended", need.recommended)] {
                if value.is_some_and(|v| !range.contains(&v)) {
                    self.fail(
                        format!("{at}.{bound}"),
                        format!("expected {} to {}", range.start(), range.end()),
                    );
                }
            }
            if let (Some(min), Some(recommended)) = (need.min, need.recommended)
                && min > recommended
            {
                self.fail(&at, "min must not exceed recommended");
            }
            if need.min.is_none() && need.recommended.is_none() {
                self.fail(&at, "expected min, recommended or both");
            }
        }
        for (key, path) in [
            ("assets.icon", &m.assets.icon),
            ("assets.cover", &m.assets.cover),
            ("assets.thumbnail", &m.assets.thumbnail),
        ] {
            if path.as_deref().is_some_and(|p| !is_prefab_path(p)) {
                self.fail(key, "expected a relative path inside the prefab");
            }
        }
        self.extensions("", m.extensions.keys());
        if m.services.is_empty() {
            self.fail("services", "at least one service is required");
        }
    }

    fn extensions<'a>(&mut self, prefix: &str, keys: impl Iterator<Item = &'a String>) {
        for key in keys {
            if !key.starts_with("x-") {
                self.fail(format!("{prefix}{key}"), "unknown key");
            }
        }
    }

    fn service(&mut self, m: &Manifest, name: &str, s: &Service) {
        let at = |field: &str| format!("services.{name}.{field}");
        if !is_key(name) {
            self.fail(format!("services.{name}"), "invalid service name");
        }
        self.extensions(&format!("services.{name}."), s.extensions.keys());
        if !IMAGE.is_match(&s.image) || s.image.ends_with(":latest") {
            self.fail(
                at("image"),
                "expected registry/name:tag or @sha256 digest, never `latest`",
            );
        }
        for key in s.env.keys() {
            if !ENV_NAME.is_match(key) {
                self.fail(at(&format!("env.{key}")), "invalid variable name");
            }
        }
        for (port, decl) in &s.ports {
            if !is_key(port) {
                self.fail(at(&format!("ports.{port}")), "invalid port name");
            }
            if decl.container == 0 {
                self.fail(at(&format!("ports.{port}.container")), "must be 1-65535");
            }
            if let Some(srv) = &decl.srv {
                if !SRV_NAME.is_match(srv) {
                    self.fail(
                        at(&format!("ports.{port}.srv")),
                        "expected _service._tcp or _service._udp",
                    );
                }
                if !decl.public {
                    self.fail(
                        at(&format!("ports.{port}.srv")),
                        "only a public port has an SRV record",
                    );
                }
            }
            // Port web (ADR-0038) : HTTP dans le conteneur, servi par le proxy du nœud, jamais
            // publié ni annoncé en SRV.
            if decl.web {
                if decl.protocol != Protocol::Tcp {
                    self.fail(at(&format!("ports.{port}.web")), "a web port is tcp");
                }
                if decl.public {
                    self.fail(
                        at(&format!("ports.{port}.web")),
                        "a web port is never public: the node proxy serves it",
                    );
                }
                if decl.srv.is_some() {
                    self.fail(
                        at(&format!("ports.{port}.web")),
                        "a web port has no SRV record",
                    );
                }
            }
        }
        for (volume, decl) in &s.volumes {
            if !is_key(volume) {
                self.fail(at(&format!("volumes.{volume}")), "invalid volume name");
            }
            if !is_abs_path(&decl.path) {
                self.fail(
                    at(&format!("volumes.{volume}.path")),
                    "expected an absolute path without `..`",
                );
            }
        }
        let mut writable = HashSet::new();
        for path in &s.writable {
            if !is_abs_path(path) || !writable.insert(path) {
                self.fail(
                    at("writable"),
                    format!("invalid or duplicate path `{path}`"),
                );
            }
        }
        if let Some(user) = &s.user
            && !USER.is_match(user)
        {
            self.fail(at("user"), "expected uid:gid with a non-zero uid");
        }
        if let Some(share) = s.resources.share
            && !(1..=100).contains(&share)
        {
            self.fail(at("resources.share"), "must be 1-100");
        }
        for dependency in &s.depends_on {
            if dependency == name || !m.services.contains_key(dependency) {
                self.fail(at("depends_on"), format!("unknown service `{dependency}`"));
            }
        }
        if let Some(health) = &s.healthcheck
            && (health.command.is_empty() || health.interval == 0 || health.retries == 0)
        {
            self.fail(
                at("healthcheck"),
                "command required, interval and retries at least 1",
            );
        }
    }

    fn resources(&mut self, m: &Manifest) {
        if m.services.len() < 2 {
            return;
        }
        let shares: Vec<Option<u8>> = m.services.values().map(|s| s.resources.share).collect();
        let total: u32 = shares.iter().flatten().map(|&s| u32::from(s)).sum();
        if shares.iter().any(Option::is_none) || total != 100 {
            self.fail(
                "services",
                "every service of a multi-service prefab needs resources.share, totalling 100",
            );
        }
    }

    fn settings(&mut self, m: &Manifest) {
        let mut keys = HashSet::new();
        for (section, groups) in [
            ("general", &m.settings.general),
            ("advanced", &m.settings.advanced),
        ] {
            for (group, def) in groups {
                let at = format!("settings.{section}.{group}");
                if !is_key(group) {
                    self.fail(&at, "invalid group name");
                }
                if def.fields.is_empty() {
                    self.fail(&at, "a group needs at least one field");
                }
                for (key, field) in &def.fields {
                    let path = format!("{at}.fields.{key}");
                    if !is_key(key) {
                        self.fail(&path, "invalid field name");
                    }
                    if !keys.insert(key.as_str()) {
                        self.fail(&path, "field declared twice (fields share one namespace)");
                    }
                    self.field(&path, field);
                }
            }
        }
    }

    fn field(&mut self, at: &str, f: &Field) {
        match f.kind {
            FieldType::Enum if f.options.as_ref().is_none_or(Vec::is_empty) => {
                self.fail(at, "an enum needs options");
            }
            FieldType::Slider if f.min.is_none() || f.max.is_none() => {
                self.fail(at, "a slider needs min and max");
            }
            _ => {}
        }
        if let Some(generate) = f.generate
            && (f.kind != FieldType::Secret || !(12..=128).contains(&generate.length))
        {
            self.fail(at, "generate applies to a secret, length 12-128");
        }
        if f.kind == FieldType::Secret && f.default.is_some() {
            self.fail(at, "a secret has no default");
        }
        if f.step.is_some_and(|step| step <= 0.0) {
            self.fail(at, "step must be positive");
        }
        if let (Some(min), Some(max)) = (f.min, f.max)
            && min > max
        {
            self.fail(at, "min is greater than max");
        }
        if f.max_length == Some(0) {
            self.fail(at, "max_length must be at least 1");
        }
        if let Some(pattern) = &f.pattern
            && Regex::new(pattern).is_err()
        {
            self.fail(at, "invalid pattern");
        }
        if let Some(default) = &f.default
            && let Err(reason) = settings::check_value(f, default)
        {
            self.fail(at, format!("invalid default: {}", reason.as_str()));
        }
    }

    fn files(&mut self, p: &Package) {
        let m = &p.manifest;
        for (id, file) in &m.files {
            let at = format!("files.{id}");
            if !is_slug(id) {
                self.fail(&at, "invalid file id");
            }
            self.under_volume(m, &file.service, &file.path, &format!("{at}.path"));
            match file.mode {
                FileMode::Template if file.template.is_none() => {
                    self.fail(&at, "mode template needs a template");
                }
                FileMode::ManagedKeys if file.keys.is_empty() => {
                    self.fail(&at, "mode managed-keys needs keys");
                }
                FileMode::ManagedKeys if !MANAGED_KEY_FORMATS.contains(&file.format) => {
                    self.fail(&at, "managed-keys supports properties, ini, json and yaml");
                }
                FileMode::ManagedKeys => {
                    let structured = STRUCTURED_KEY_FORMATS.contains(&file.format);
                    for (key, value) in &file.keys {
                        let at_key = format!("{at}.keys.{key}");
                        let valid_key = if structured {
                            !key.is_empty()
                                && key.split('.').all(|segment| {
                                    !segment.is_empty()
                                        && segment.chars().all(|c| {
                                            c.is_ascii_alphanumeric() || c == '_' || c == '-'
                                        })
                                })
                        } else {
                            !key.is_empty() && !key.contains(['\n', '=', ' '])
                        };
                        if !valid_key {
                            self.fail(&at_key, "invalid key");
                        }
                        if !is_managed_value(value) {
                            self.fail(
                                &at_key,
                                "expected a string, number, boolean or list of those",
                            );
                        } else if !structured && !value.is_string() {
                            self.fail(&at_key, "only strings in a properties or ini file");
                        }
                    }
                }
                _ => {}
            }
            if let Some(template) = &file.template {
                self.template_file(p, template, &format!("{at}.template"));
            }
        }
    }

    fn template_file(&mut self, p: &Package, path: &str, at: &str) {
        if !is_prefab_path(path) {
            self.fail(at, "expected a relative path inside the prefab");
        } else if !p.templates.contains_key(path) {
            self.fail(at, format!("missing {path}"));
        }
    }

    fn install(&mut self, p: &Package) {
        let m = &p.manifest;
        let Some(install) = &m.install else { return };
        if let Some(image) = &install.image
            && (!IMAGE.is_match(image) || image.ends_with(":latest"))
        {
            self.fail(
                "install.image",
                "expected registry/name:tag or @sha256 digest, never `latest`",
            );
        }
        if install.steps.is_empty() {
            self.fail("install.steps", "at least one step is required");
        }
        for (index, step) in install.steps.iter().enumerate() {
            let at = format!("install.steps[{index}]");
            match step {
                Step::Download { url, sha256, to } => {
                    if !is_public_https_url(url) {
                        self.fail(
                            &at,
                            "url must be https and must not point to a local or private host",
                        );
                    }
                    if !SHA256.is_match(sha256) {
                        self.fail(&at, "sha256 must be 64 lowercase hex characters");
                    }
                    self.under_volume(m, &install.service, to, &at);
                }
                Step::Extract {
                    from,
                    to,
                    subdir,
                    executable,
                } => {
                    self.under_volume(m, &install.service, from, &at);
                    self.under_volume(m, &install.service, to, &at);
                    for relative in subdir.iter().chain(executable) {
                        if !is_prefab_path(relative) {
                            self.fail(
                                &at,
                                format!("`{relative}`: expected a relative path without `..`"),
                            );
                        }
                    }
                }
                Step::Remove { path } => self.under_volume(m, &install.service, path, &at),
                Step::Copy { template, to } => {
                    self.template_file(p, template, &at);
                    self.under_volume(m, &install.service, to, &at);
                }
                Step::Steamcmd { dir, app, .. } => {
                    if *app == 0 {
                        self.fail(&at, "app must be a Steam app id");
                    }
                    self.under_volume(m, &install.service, dir, &at);
                }
                Step::Script { image, .. } => {
                    if image
                        .as_deref()
                        .is_some_and(|i| !IMAGE.is_match(i) || i.ends_with(":latest"))
                    {
                        self.fail(&at, "invalid image");
                    }
                }
            }
        }
    }

    fn backup(&mut self, m: &Manifest) {
        let Some(backup) = &m.backup else { return };
        for (service, patterns) in &backup.ignore {
            for (index, pattern) in patterns.iter().enumerate() {
                let at = format!("backup.ignore.{service}[{index}]");
                if pattern
                    .split('/')
                    .any(|part| part.contains("**") && part != "**")
                {
                    self.fail(&at, "`**` must be a whole path segment");
                }
                // La partie fixe du motif (avant le premier joker) doit être sous un volume.
                let fixed = pattern
                    .split('/')
                    .take_while(|part| !part.contains('*'))
                    .collect::<Vec<_>>()
                    .join("/");
                self.under_volume(m, service, &fixed, &at);
            }
        }
        for (list, hooks) in [("before", &backup.before), ("after", &backup.after)] {
            for (index, hook) in hooks.iter().enumerate() {
                let at = format!("backup.{list}[{index}]");
                if !kuro_protocol::limits::is_valid_command(&hook.console) {
                    self.fail(&at, "invalid console command");
                }
                if hook.wait.as_deref().is_some_and(|w| Regex::new(w).is_err()) {
                    self.fail(&at, "invalid wait regex");
                }
            }
        }
    }

    fn content(&mut self, m: &Manifest) {
        for (slot, def) in &m.content {
            let at = format!("content.{slot}");
            if !is_key(slot) {
                self.fail(&at, "invalid slot name");
            }
            if def.providers.is_empty() {
                self.fail(&at, "at least one provider is required");
            }
            match (def.install.mode, &def.install.dir) {
                (ContentInstallMode::Download, None) => self.fail(&at, "mode download needs a dir"),
                (_, Some(dir)) => {
                    self.under_volume(m, &def.service, dir, &format!("{at}.install.dir"))
                }
                _ => {}
            }
            if let Some(path) = &def.install.load_order {
                self.under_volume(m, &def.service, path, &format!("{at}.install.load_order"));
            }
            for (field, spec) in &def.fields {
                if !is_key(field) {
                    self.fail(format!("{at}.fields.{field}"), "invalid field name");
                }
                if let Some(prefill) = &spec.prefill {
                    match prefill.pattern() {
                        Ok(pattern) if pattern.captures_len() >= 2 => {}
                        Ok(_) => self.fail(
                            format!("{at}.fields.{field}.prefill.description"),
                            "the pattern needs a capture group",
                        ),
                        Err(_) => self.fail(
                            format!("{at}.fields.{field}.prefill.description"),
                            "invalid regular expression",
                        ),
                    }
                }
            }
            // Les valeurs servent aux gabarits, rendus au déploiement : le mode `reference`
            // redéploie à chaque changement, les autres non.
            if !def.fields.is_empty() && def.install.mode != ContentInstallMode::Reference {
                self.fail(format!("{at}.fields"), "fields need install mode reference");
            }
            self.extraction(m, slot, def);
        }
    }

    /// Mode `extract` (ADR-0030) : description et routes des paquets, fichiers d'inscription.
    fn extraction(&mut self, m: &Manifest, slot: &str, def: &crate::manifest::ContentSlot) {
        let at = format!("content.{slot}");
        let extract = def.install.mode == ContentInstallMode::Extract;
        if extract {
            match def.install.describe.as_deref() {
                Some(describe) if is_prefab_path(describe) => {}
                _ => self.fail(
                    format!("{at}.install.describe"),
                    "mode extract needs a relative description file (manifest.json)",
                ),
            }
            if def.install.route.is_empty() {
                self.fail(
                    format!("{at}.install.route"),
                    "mode extract needs at least one route",
                );
            }
            if def
                .providers
                .iter()
                .any(|p| p.provider == crate::manifest::Provider::Upload)
            {
                self.fail(
                    format!("{at}.providers"),
                    "upload is not supported with install mode extract yet",
                );
            }
        } else if def.install.describe.is_some() || !def.install.route.is_empty() {
            self.fail(
                format!("{at}.install"),
                "describe and route need install mode extract",
            );
        }
        for (index, route) in def.install.route.iter().enumerate() {
            let route_at = format!("{at}.install.route[{index}]");
            self.under_volume(m, &def.service, &route.dir, &format!("{route_at}.dir"));
            self.condition(&route.when, &format!("{route_at}.when"));
        }
        if !def.register.is_empty() && !extract {
            self.fail(
                format!("{at}.register"),
                "register needs install mode extract",
            );
        }
        for (index, register) in def.register.iter().enumerate() {
            let register_at = format!("{at}.register[{index}]");
            // Chemin rendu au déploiement : la partie fixe, avant le premier gabarit, sous un volume.
            let fixed = register.file.split("{{").next().unwrap_or_default();
            let fixed = fixed.rsplit_once('/').map_or(fixed, |(dir, _)| dir);
            if !register.file.ends_with(".json") {
                self.fail(format!("{register_at}.file"), "expected a .json file");
            }
            self.under_volume(m, &def.service, fixed, &format!("{register_at}.file"));
            if let Some(when) = &register.when {
                self.condition(when, &format!("{register_at}.when"));
            }
            if register.entry.is_empty() {
                self.fail(format!("{register_at}.entry"), "expected at least one key");
            }
            for (key, template) in &register.entry {
                if template.contains("{{") && crate::resolve::item_reference(template).is_none() {
                    self.fail(
                        format!("{register_at}.entry.{key}"),
                        "expected fixed text or exactly `{{ item.<path> }}`",
                    );
                }
            }
        }
    }

    fn condition(&mut self, when: &crate::manifest::ContentWhen, at: &str) {
        if when.value.is_empty() || when.value.split('.').any(str::is_empty) || when.is.is_empty() {
            self.fail(at, "expected a value path (`modules.0.type`) and a value");
        }
    }

    /// Migrations de réglages (ADR-0024) : contrainte `from` valide, renommages vers des champs
    /// de cette version, pas de clé à la fois renommée et retirée.
    fn migrations(&mut self, m: &Manifest) {
        for (index, migration) in m.migrations.iter().enumerate() {
            let at = format!("migrations.{index}");
            if semver::VersionReq::parse(&migration.from).is_err() {
                self.fail(format!("{at}.from"), "invalid semver requirement");
            }
            for (old, new) in &migration.rename {
                if m.settings.field(new).is_none() {
                    self.fail(
                        format!("{at}.rename.{old}"),
                        format!("unknown setting {new} in this version"),
                    );
                }
                if migration.remove.contains(old) {
                    self.fail(
                        format!("{at}.rename.{old}"),
                        "a setting cannot be both renamed and removed",
                    );
                }
            }
        }
    }

    fn lifecycle(&mut self, m: &Manifest) {
        let l = &m.lifecycle;
        let Some(service) = m.services.get(&l.service) else {
            self.fail(
                "lifecycle.service",
                format!("unknown service `{}`", l.service),
            );
            return;
        };
        match &l.ready {
            Some(Ready::Console(pattern)) if Regex::new(pattern).is_err() => {
                self.fail("lifecycle.ready.console", "invalid regex");
            }
            Some(Ready::Healthcheck(flag)) if !flag || service.healthcheck.is_none() => {
                self.fail(
                    "lifecycle.ready.healthcheck",
                    "needs `true` and a healthcheck on the service",
                );
            }
            _ => {}
        }
        if !(1..=600).contains(&l.timeout) {
            self.fail("lifecycle.timeout", "must be 1-600 seconds");
        }
    }

    /// Chemin d'un service, obligatoirement sous l'un de ses volumes.
    fn under_volume(&mut self, m: &Manifest, service: &str, path: &str, at: &str) {
        let Some(s) = m.services.get(service) else {
            self.fail(at, format!("unknown service `{service}`"));
            return;
        };
        if !is_abs_path(path) {
            self.fail(at, "expected an absolute path without `..`");
            return;
        }
        let inside = s.volumes.values().any(|v| {
            let root = v.path.trim_end_matches('/');
            path == root || path.starts_with(&format!("{root}/"))
        });
        if !inside {
            self.fail(
                at,
                format!("`{path}` is not under a volume of service `{service}`"),
            );
        }
    }

    /// Actions (ADR-0039) : service connu, commande non vide, délai borné, paramètres typés
    /// comme des settings (ni secret, ni mot de passe, ni généré), et chaque paramètre entre dans
    /// la commande comme argument entier, jamais concaténé à autre chose.
    fn actions(&mut self, m: &Manifest) {
        let fields: BTreeSet<&str> = m.settings.fields().map(|f| f.key).collect();
        let public_ports: BTreeSet<&str> = m
            .services
            .values()
            .flat_map(|s| {
                s.ports
                    .iter()
                    .filter(|(_, p)| p.public)
                    .map(|(n, _)| n.as_str())
            })
            .collect();
        for (key, action) in &m.actions {
            let at = format!("actions.{key}");
            if !is_slug(key) {
                self.fail(&at, "invalid action key");
            }
            if !m.services.contains_key(&action.service) {
                self.fail(format!("{at}.service"), "unknown service");
            }
            if action.command.is_empty() {
                self.fail(format!("{at}.command"), "a command is required");
            }
            if action.timeout == 0 || action.timeout > MAX_ACTION_TIMEOUT_SECS {
                self.fail(
                    format!("{at}.timeout"),
                    format!("expected 1 to {MAX_ACTION_TIMEOUT_SECS} seconds"),
                );
            }
            for (name, field) in &action.params {
                let at = format!("{at}.params.{name}");
                if !is_key(name) {
                    self.fail(&at, "invalid parameter name");
                }
                if !matches!(
                    field.kind,
                    FieldType::String
                        | FieldType::Int
                        | FieldType::Float
                        | FieldType::Bool
                        | FieldType::Enum
                ) {
                    self.fail(&at, "a parameter is a string, int, float, bool or enum");
                }
                if field.generate.is_some() {
                    self.fail(&at, "a parameter is never generated");
                }
                if field.kind == FieldType::String && field.pattern.is_none() {
                    self.fail(&at, "a string parameter needs a pattern");
                }
                self.field(&at, field);
            }
            if action.schedulable
                && action
                    .params
                    .values()
                    .any(|f| f.required && f.default.is_none())
            {
                self.fail(
                    format!("{at}.schedulable"),
                    "a schedulable action has no required parameter",
                );
            }
            for (index, arg) in action.command.iter().enumerate() {
                let at = format!("{at}.command[{index}]");
                let template = match Template::parse(arg) {
                    Ok(template) => template,
                    Err(error) => {
                        self.fail(&at, error.to_string());
                        continue;
                    }
                };
                for reference in template.references() {
                    let problem = match reference {
                        Reference::Param(name) if !action.params.contains_key(name) => {
                            Some(format!("unknown parameter `{name}`"))
                        }
                        Reference::Param(_) if !is_whole_expression(arg) => {
                            Some("a parameter is a whole argument, never part of one".to_owned())
                        }
                        Reference::Setting(key) if !fields.contains(key.as_str()) => {
                            Some(format!("unknown setting `{key}`"))
                        }
                        Reference::PortHost(port) if !public_ports.contains(port.as_str()) => {
                            Some(format!("`{port}` is not a public port"))
                        }
                        Reference::ContentIds(_) | Reference::ContentValues(..) => {
                            Some("content lists have no place in an action".to_owned())
                        }
                        _ => None,
                    };
                    if let Some(problem) = problem {
                        self.fail(&at, problem);
                    }
                }
            }
        }
    }

    /// Un seul port web par prefab (ADR-0038).
    fn web_ports(&mut self, m: &Manifest) {
        let web: Vec<String> = m
            .services
            .iter()
            .flat_map(|(service, s)| {
                s.ports
                    .iter()
                    .filter(|(_, decl)| decl.web)
                    .map(move |(port, _)| format!("services.{service}.ports.{port}"))
            })
            .collect();
        if web.len() > 1 {
            for at in web {
                self.fail(at, "only one web port per prefab");
            }
        }
    }

    /// Toutes les valeurs à gabarit : syntaxe et références.
    fn templates(&mut self, m: &Manifest) {
        let fields: BTreeSet<&str> = m.settings.fields().map(|f| f.key).collect();
        let has_web_port = m.web_port().is_some();
        let public_ports: BTreeSet<&str> = m
            .services
            .values()
            .flat_map(|s| {
                s.ports
                    .iter()
                    .filter(|(_, p)| p.public)
                    .map(|(n, _)| n.as_str())
            })
            .collect();
        let all_ports: BTreeSet<&str> = m
            .services
            .values()
            .flat_map(|s| s.ports.keys().map(String::as_str))
            .collect();

        let mut sources: Vec<(String, &str)> = Vec::new();
        for (name, s) in &m.services {
            for (index, arg) in s.command.iter().enumerate() {
                sources.push((format!("services.{name}.command[{index}]"), arg));
            }
            for (key, value) in &s.env {
                sources.push((format!("services.{name}.env.{key}"), value));
            }
            if let Some(health) = &s.healthcheck {
                for (index, arg) in health.command.iter().enumerate() {
                    sources.push((format!("services.{name}.healthcheck.command[{index}]"), arg));
                }
            }
        }
        for (id, file) in &m.files {
            for (key, value) in &file.keys {
                match value {
                    Value::String(text) => {
                        sources.push((format!("files.{id}.keys.{key}"), text.as_str()));
                    }
                    Value::Array(items) => {
                        for (index, item) in items.iter().enumerate() {
                            if let Value::String(text) = item {
                                sources.push((
                                    format!("files.{id}.keys.{key}[{index}]"),
                                    text.as_str(),
                                ));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        for (slot, def) in &m.content {
            for provider in &def.providers {
                if let Some(loader) = &provider.loader {
                    sources.push((format!("content.{slot}.providers.loader"), loader));
                }
                if let Some(version) = &provider.game_version {
                    sources.push((format!("content.{slot}.providers.game_version"), version));
                }
            }
        }

        for (at, source) in sources {
            let template = match Template::parse(source) {
                Ok(template) => template,
                Err(error) => {
                    self.fail(at, error.to_string());
                    continue;
                }
            };
            for reference in template.references() {
                let problem = match reference {
                    Reference::Setting(key) if !fields.contains(key.as_str()) => {
                        Some(format!("unknown setting `{key}`"))
                    }
                    Reference::PortHost(port) if !public_ports.contains(port.as_str()) => {
                        Some(format!("`{port}` is not a public port"))
                    }
                    Reference::PortContainer(port) if !all_ports.contains(port.as_str()) => {
                        Some(format!("unknown port `{port}`"))
                    }
                    Reference::ServerWebUrl | Reference::ServerWebHost if !has_web_port => {
                        Some(format!("`{reference}` needs a `web: true` port"))
                    }
                    Reference::Param(_) => {
                        Some("`params` are only available in an action command".to_owned())
                    }
                    Reference::ContentIds(slot) if !m.content.contains_key(slot) => {
                        Some(format!("unknown content slot `{slot}`"))
                    }
                    Reference::ContentIds(_) if !template.joins(reference) => {
                        Some(format!("`{reference}` is a list: use `join`"))
                    }
                    Reference::ContentValues(slot, field)
                        if !m
                            .content
                            .get(slot)
                            .is_some_and(|def| def.fields.contains_key(field)) =>
                    {
                        Some(format!("unknown content field `{slot}.{field}`"))
                    }
                    Reference::ContentValues(..) if !template.joins(reference) => {
                        Some(format!("`{reference}` is a list: use `join`"))
                    }
                    _ => None,
                };
                if let Some(problem) = problem {
                    self.fail(&at, problem);
                }
            }
        }
    }

    /// Mêmes clés exigées dans chaque langue ; l'anglais sert de repli.
    fn locales(&mut self, p: &Package) {
        let m = &p.manifest;
        if !p.locales.contains_key("en") {
            self.fail("locales", "locales/en.json is required (fallback language)");
        }
        let mut required = vec!["prefab.name".to_owned(), "prefab.description".to_owned()];
        let mut groups = BTreeSet::new();
        for f in m.settings.fields() {
            groups.insert(f.group);
            required.push(format!("fields.{}.label", f.key));
            if f.field.kind == FieldType::Enum {
                for option in f.field.options.iter().flatten() {
                    required.push(format!(
                        "fields.{}.options.{}",
                        f.key,
                        crate::template::scalar(option)
                    ));
                }
            }
        }
        required.extend(groups.into_iter().map(|g| format!("groups.{g}.title")));
        required.extend(m.files.keys().map(|id| format!("files.{id}.description")));
        required.extend(m.content.keys().map(|slot| format!("content.{slot}.title")));
        for (key, action) in &m.actions {
            required.push(format!("actions.{key}.label"));
            required.extend(
                action
                    .params
                    .keys()
                    .map(|name| format!("actions.{key}.params.{name}.label")),
            );
        }
        for (slot, def) in &m.content {
            required.extend(
                def.fields
                    .keys()
                    .map(|field| format!("content.{slot}.fields.{field}.label")),
            );
        }

        for (lang, messages) in &p.locales {
            for key in &required {
                let found = key
                    .split('.')
                    .try_fold(messages, |node, part| node.get(part))
                    .is_some_and(|value| matches!(value, Value::String(_)));
                if !found {
                    self.fail(format!("locales/{lang}.json"), format!("missing `{key}`"));
                }
            }
        }
    }
}

/// Chemin absolu dans un conteneur, sans `..` ni caractère nul.
pub fn is_abs_path(path: &str) -> bool {
    path.starts_with('/') && !path.contains('\0') && !path.split('/').any(|part| part == "..")
}

/// Chemin relatif à la racine du prefab, sans `..`.
pub fn is_prefab_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != ".."
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        })
}

/// URL de téléchargement acceptable : `https://`, sans identifiants, vers un hôte qui n'est ni
/// local ni une adresse privée écrite en clair. Le nœud revérifie après résolution DNS : ceci
/// refuse tôt, au chargement du prefab, ce qui ne pourra jamais passer.
fn is_public_https_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(v6),
        None => authority
            .rsplit_once(':')
            .filter(|(_, port)| !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()))
            .map_or(authority, |(host, _)| host),
    };
    let host = host.to_ascii_lowercase();
    if host.is_empty()
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
    {
        return false;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || a == 0
                || a >= 224
                || (a == 100 && (64..128).contains(&b)))
        }
        Ok(std::net::IpAddr::V6(v6)) => {
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || v6.to_ipv4_mapped().is_some())
        }
        Err(_) => true,
    }
}

#[cfg(test)]
mod download_url_tests {
    use super::is_public_https_url;

    #[test]
    fn only_public_https_hosts_are_accepted() {
        for url in [
            "https://piston-data.mojang.com/v1/objects/x/server.jar",
            "https://cdn.example.com:8443/file.tar.gz?token=1",
            "https://[2001:4860:4860::8888]/file",
        ] {
            assert!(is_public_https_url(url), "{url}");
        }
        for url in [
            "http://piston-data.mojang.com/server.jar",
            "https://localhost/server.jar",
            "https://build.localhost/server.jar",
            "https://127.0.0.1:8443/server.jar",
            "https://10.0.0.5/server.jar",
            "https://169.254.169.254/latest/meta-data",
            "https://[::1]/server.jar",
            "https://[fd00::1]/server.jar",
            "https://user:pw@cdn.example.com/server.jar",
            "https:///server.jar",
            "ftp://cdn.example.com/server.jar",
        ] {
            assert!(!is_public_https_url(url), "{url}");
        }
    }
}
