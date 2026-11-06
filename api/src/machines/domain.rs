//! Règles des machines louées (ADR-0065), sans entrée ni sortie : décision du ménage, choix de
//! l'offre, cloud-init d'enrôlement, nom neutre.

use std::time::Duration;

use time::OffsetDateTime;
use uuid::Uuid;

use super::provider::Offer;

/// Étiquette posée chez le fournisseur, valeur = identifiant de la machine.
pub const LABEL: &str = "kuro-machine";
/// Étiquette de l'instance de Kuro (`machine_settings.instance_tag`).
pub const SCOPE_LABEL: &str = "kuro-instance";
/// Jeton d'enrôlement d'une machine : démarrage de la VM, Docker et daemon compris.
pub const ENROLL_TTL: Duration = Duration::from_secs(30 * 60);
/// Délai pendant lequel une machine est « en cours de création » : jamais abandonnée ni
/// supprimée comme orpheline avant.
pub const GRACE: time::Duration = time::Duration::minutes(30);
/// Nœud jamais en ligne après ce délai : machine bloquée.
pub const STALL: time::Duration = time::Duration::minutes(45);
/// Installeur du nœud de Kuro (rôle `node` pré-choisi).
pub const INSTALLER_URL: &str = "https://get.kuroctl.com/node";
const TOKEN_FILE: &str = "/root/kuro-enroll-token";
/// Disque laissé au système, à Docker et au daemon de Kuro.
pub const DISK_RESERVE_GB: u32 = 10;

/// Mémoire laissée au système, à Docker et au daemon de Kuro : un quart de la machine, entre
/// 256 Mo et 1 Go (une machine de 1 Go garde 768 Mo pour son serveur).
pub fn memory_reserve_mb(total_mb: u32) -> u32 {
    (total_mb / 4).clamp(256, 1024)
}

/// État d'une machine (colonne `machines.state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineState {
    /// Ligne écrite avant l'appel au fournisseur.
    Creating,
    Ready,
    /// Snapshot demandé, VM encore là.
    Sleeping,
    /// VM rendue, snapshot gardé.
    Asleep,
    /// VM recréée depuis le snapshot, nœud attendu.
    Waking,
    /// Rendue en cours, rejouée par le ménage.
    Releasing,
    Deleted,
}

impl MachineState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Creating => "creating",
            Self::Ready => "ready",
            Self::Sleeping => "sleeping",
            Self::Asleep => "asleep",
            Self::Waking => "waking",
            Self::Releasing => "releasing",
            Self::Deleted => "deleted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "creating" => Self::Creating,
            "ready" => Self::Ready,
            "sleeping" => Self::Sleeping,
            "asleep" => Self::Asleep,
            "waking" => Self::Waking,
            "releasing" => Self::Releasing,
            "deleted" => Self::Deleted,
            _ => return None,
        })
    }
}

/// Ce qu'un passage fait d'une machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Keep,
    /// Ligne `creating` ou `waking` dont la VM existe : rattachée (référence, adresse).
    Adopt,
    /// Ligne `creating` sans VM après le délai : nœud retiré, ligne close. `waking` : retour
    /// au sommeil (le snapshot est toujours là).
    Abandon,
    /// VM étiquetée sans ligne vivante, après le délai : supprimée chez le fournisseur.
    Orphan,
    /// Ligne `ready` ou `sleeping` dont la VM a disparu : signalée, rien n'est fait.
    Lost,
    /// Nœud jamais en ligne : rendue sans `claim`, signalée sinon.
    Stalled,
    /// Rendue interrompue : reprise.
    FinishRelease,
}

/// Ligne de `machines` vue par le passage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowState {
    pub state: MachineState,
    pub created_at: OffsetDateTime,
    /// Dernier changement d'état.
    pub since: OffsetDateTime,
    /// Le nœud a déjà été vu en ligne.
    pub node_seen: bool,
    /// La référence de la VM est notée (réponse du fournisseur reçue).
    pub has_reference: bool,
}

/// Action d'une machine, d'après sa ligne (`None` : absente ou close) et la date de création de
/// sa VM chez le fournisseur (`None` : pas de VM).
pub fn machine_step(
    row: Option<RowState>,
    vm: Option<OffsetDateTime>,
    now: OffsetDateTime,
) -> Action {
    use MachineState as S;
    let Some(row) = row else {
        return match vm {
            Some(created_at) if now - created_at >= GRACE => Action::Orphan,
            _ => Action::Keep,
        };
    };
    match (row.state, vm) {
        (S::Releasing, _) => Action::FinishRelease,
        (S::Creating | S::Waking, Some(_)) if !row.has_reference => Action::Adopt,
        (S::Creating, None) if now - row.created_at >= GRACE => Action::Abandon,
        (S::Waking, None) if now - row.since >= GRACE => Action::Abandon,
        (S::Ready | S::Sleeping, None) => Action::Lost,
        (S::Ready, Some(_)) if !row.node_seen && now - row.created_at >= STALL => Action::Stalled,
        _ => Action::Keep,
    }
}

/// Ressources à couvrir (celles d'un serveur ou d'une formule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resources {
    pub memory_mb: i32,
    /// 100 = un cœur.
    pub cpu_percent: i32,
    pub disk_mb: i32,
}

/// Offre la moins chère qui couvre les ressources, réserve du système comprise.
pub fn cheapest_covering(offers: &[Offer], need: Resources) -> Option<&Offer> {
    let vcpus = u32::try_from(need.cpu_percent).ok()?.div_ceil(100).max(1);
    let memory = u32::try_from(need.memory_mb).ok()?;
    let disk = u32::try_from(need.disk_mb).ok()?.div_ceil(1024) + DISK_RESERVE_GB;
    offers
        .iter()
        .filter(|o| {
            o.vcpus >= vcpus
                && o.memory_mb.saturating_sub(memory_reserve_mb(o.memory_mb)) >= memory
                && o.disk_gb >= disk
        })
        .min_by_key(|o| o.hourly_cost)
}

/// Document cloud-init : jeton dans un fichier (jamais en argument), installeur du nœud de Kuro
/// sur une machine vierge. Chaque valeur est une chaîne JSON, donc du YAML valide quel que soit
/// son contenu.
pub fn cloud_init(api_url: &str, fingerprint: &str, token: &str, ports: &str) -> String {
    let q = |value: &str| serde_json::Value::from(value).to_string();
    let install = [
        "bash",
        "/root/kuro-install.sh",
        "node",
        "--api",
        api_url,
        "--ca-fingerprint",
        fingerprint,
        "--token-file",
        TOKEN_FILE,
        "--ports",
        ports,
        "--full",
        "--yes",
    ]
    .map(q)
    .join(", ");
    format!(
        "#cloud-config\n\
         write_files:\n  \
           - path: {path}\n    \
             permissions: '0600'\n    \
             content: {token}\n\
         runcmd:\n  \
           - [curl, -fsSLo, /root/kuro-install.sh, {url}]\n  \
           - [{install}]\n  \
           - [rm, -f, {path}]\n",
        path = q(TOKEN_FILE),
        token = q(token),
        url = q(INSTALLER_URL),
    )
}

/// Nom neutre de la machine et de son nœud : partie aléatoire de l'identifiant.
pub fn machine_name(id: Uuid) -> String {
    let hex = id.simple().to_string();
    format!("m-{}", &hex[20..])
}

/// Emplacement chez un fournisseur (`fsn1`, `nbg1`, `ash`).
pub fn is_location(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_init_keeps_the_token_out_of_the_command_line() {
        let doc = cloud_init(
            "https://panel.example.com:8443",
            "sha256:abc",
            "kuro_enr_secret\"\nx",
            "25565-25600",
        );
        assert!(doc.starts_with("#cloud-config\n"));
        assert!(doc.contains(r#"content: "kuro_enr_secret\"\nx""#));
        let install = doc
            .lines()
            .find(|line| line.contains("kuro-install.sh\", \"node\""))
            .unwrap_or_default();
        assert!(install.contains(r#""--token-file", "/root/kuro-enroll-token""#));
        assert!(install.contains(r#""--api", "https://panel.example.com:8443""#));
        assert!(!install.contains("kuro_enr_secret"));
    }

    #[test]
    fn machine_step_decides_from_the_row_and_the_vm() {
        use MachineState as S;
        let now = OffsetDateTime::now_utc();
        let young = now - time::Duration::minutes(5);
        let old = now - time::Duration::minutes(31);
        let older = now - time::Duration::minutes(46);
        let row = |state, created_at, since, node_seen| {
            Some(RowState {
                state,
                created_at,
                since,
                node_seen,
                has_reference: !matches!(state, S::Creating | S::Waking),
            })
        };
        let answered = Some(RowState {
            state: S::Waking,
            created_at: older,
            since: young,
            node_seen: true,
            has_reference: true,
        });
        let cases = [
            (
                "creating, vm found",
                row(S::Creating, young, young, false),
                Some(young),
                Action::Adopt,
            ),
            (
                "creating, vm found late",
                row(S::Creating, old, old, false),
                Some(old),
                Action::Adopt,
            ),
            (
                "creating, no vm yet",
                row(S::Creating, young, young, false),
                None,
                Action::Keep,
            ),
            (
                "creating, no vm after grace",
                row(S::Creating, old, old, false),
                None,
                Action::Abandon,
            ),
            (
                "ready, vm",
                row(S::Ready, old, old, false),
                Some(old),
                Action::Keep,
            ),
            (
                "ready, vm gone",
                row(S::Ready, old, old, true),
                None,
                Action::Lost,
            ),
            (
                "ready, node never online",
                row(S::Ready, older, older, false),
                Some(older),
                Action::Stalled,
            ),
            (
                "ready, node seen",
                row(S::Ready, older, older, true),
                Some(older),
                Action::Keep,
            ),
            (
                "sleeping, vm gone",
                row(S::Sleeping, old, old, true),
                None,
                Action::Lost,
            ),
            (
                "asleep, no vm",
                row(S::Asleep, older, old, true),
                None,
                Action::Keep,
            ),
            (
                "asleep, vm being deleted",
                row(S::Asleep, older, young, true),
                Some(older),
                Action::Keep,
            ),
            (
                "waking, vm found",
                row(S::Waking, older, young, true),
                Some(young),
                Action::Adopt,
            ),
            (
                "waking, answer received",
                answered,
                Some(young),
                Action::Keep,
            ),
            (
                "waking, no vm yet",
                row(S::Waking, older, young, true),
                None,
                Action::Keep,
            ),
            (
                "waking, no vm after grace",
                row(S::Waking, older, old, true),
                None,
                Action::Abandon,
            ),
            (
                "releasing",
                row(S::Releasing, old, old, true),
                Some(old),
                Action::FinishRelease,
            ),
            ("no row, young vm", None, Some(young), Action::Keep),
            ("no row, old vm", None, Some(old), Action::Orphan),
            ("nothing", None, None, Action::Keep),
        ];
        for (case, row, vm, expected) in cases {
            assert_eq!(machine_step(row, vm, now), expected, "{case}");
        }
    }

    #[test]
    fn the_cheapest_offer_covering_the_resources_is_chosen() {
        let offer = |name: &str, vcpus, memory_mb, disk_gb, hourly_cost| Offer {
            name: name.to_owned(),
            vcpus,
            memory_mb,
            disk_gb,
            hourly_cost,
            monthly_cost: 0,
        };
        let offers = [
            offer("large", 4, 8192, 160, 2000),
            offer("small", 2, 4096, 40, 600),
            offer("tiny", 1, 2048, 20, 400),
        ];
        let need = |memory_mb, cpu_percent, disk_mb| Resources {
            memory_mb,
            cpu_percent,
            disk_mb,
        };
        let name = |need| cheapest_covering(&offers, need).map(|o| o.name.as_str());
        assert_eq!(name(need(1024, 100, 10_240)), Some("tiny"));
        // 3 Go + 1 Go de réserve tiennent tout juste dans 4 Go ; un méga de plus, non.
        assert_eq!(name(need(3072, 100, 10_240)), Some("small"));
        assert_eq!(name(need(3073, 100, 10_240)), Some("large"));
        // 150 % de CPU : deux cœurs.
        assert_eq!(name(need(1024, 150, 10_240)), Some("small"));
        // Disque : 30 Go + 10 de réserve.
        assert_eq!(name(need(1024, 100, 30 * 1024 + 1)), Some("large"));
        assert_eq!(name(need(16_384, 100, 10_240)), None);
    }

    #[test]
    fn machine_names_are_neutral_and_distinct() {
        let a = machine_name(Uuid::now_v7());
        let b = machine_name(Uuid::now_v7());
        assert!(a.starts_with("m-") && a.len() == 14);
        assert_ne!(a, b);
    }

    #[test]
    fn the_memory_reserve_follows_the_machine_size() {
        assert_eq!(memory_reserve_mb(1024), 256);
        assert_eq!(memory_reserve_mb(2048), 512);
        assert_eq!(memory_reserve_mb(16_384), 1024);
    }

    #[test]
    fn locations_are_checked() {
        assert!(is_location("fsn1"));
        assert!(!is_location("FSN1"));
        assert!(!is_location(""));
    }
}
