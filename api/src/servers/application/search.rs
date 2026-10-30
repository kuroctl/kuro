//! Liste des serveurs côté API (ADR-0025) : recherche, filtres, tri, facettes et pagination.
//!
//! Les colonnes légères de tous les serveurs sont lues en une requête, puis filtrées, comptées
//! et triées ici ; seule la page affichée est relue complète. Le calcul « mise à jour
//! disponible » (comparaison semver avec la dernière version du prefab) se fait au même endroit.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use uuid::Uuid;

use super::{ServerView, ServersError, ServersState};
use crate::prefabs::application as prefabs;
use crate::servers::infra::{ServerIndexRow, ServerRepository};

/// Taille de page maximale.
pub const MAX_PER_PAGE: usize = 100;
/// Identifiants rendus au plus pour « tout sélectionner ».
pub const MAX_IDS: usize = 1000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortKey {
    #[default]
    Name,
    State,
    Prefab,
    Node,
    Memory,
    Cpu,
    Disk,
    Created,
}

/// Critères de la liste. Un filtre vide ne restreint rien ; plusieurs valeurs : l'une d'elles.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// Nom, identifiant, nœud ou prefab contenant ce texte (sans casse).
    pub q: Option<String>,
    pub prefabs: Vec<String>,
    pub nodes: Vec<Uuid>,
    pub states: Vec<String>,
    /// Seulement ceux qui ont (`true`) ou n'ont pas (`false`) de version plus récente.
    pub upgrade: Option<bool>,
    pub suspended: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct Search {
    pub filter: Filter,
    pub sort: SortKey,
    pub descending: bool,
    /// À partir de 1.
    pub page: usize,
    pub per_page: usize,
    /// Rendre aussi les identifiants de tous les résultats (« tout sélectionner »).
    pub all_ids: bool,
}

/// Valeur d'une facette et nombre de serveurs, les autres filtres appliqués.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetValue {
    pub value: String,
    pub label: String,
    pub count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Facets {
    pub prefab: Vec<FacetValue>,
    pub node: Vec<FacetValue>,
    pub state: Vec<FacetValue>,
    pub upgrade: usize,
    pub suspended: usize,
}

#[derive(Debug, Clone)]
pub struct Page {
    pub items: Vec<ServerView>,
    pub total: usize,
    pub facets: Facets,
    /// Dernière version de chaque prefab présent dans la page, si plus récente.
    pub latest: HashMap<Uuid, String>,
    /// Tous les résultats, si demandé (au plus [`MAX_IDS`]).
    pub ids: Option<Vec<Uuid>>,
}

/// Dernière version (semver) de chaque prefab du catalogue, versions d'essai exclues.
pub async fn latest_versions(
    state: &ServersState,
) -> Result<HashMap<String, String>, ServersError> {
    let mut latest: HashMap<String, (semver::Version, String)> = HashMap::new();
    for prefab in prefabs::list(&state.pool).await? {
        let Ok(version) = prefab.version.parse::<semver::Version>() else {
            continue;
        };
        match latest.get(&prefab.id) {
            Some((best, _)) if *best >= version => {}
            _ => {
                latest.insert(prefab.id.clone(), (version, prefab.version));
            }
        }
    }
    Ok(latest
        .into_iter()
        .map(|(id, (_, text))| (id, text))
        .collect())
}

/// Version plus récente que celle du serveur, s'il y en a une.
pub fn newer<'a>(latest: &'a HashMap<String, String>, row: &ServerIndexRow) -> Option<&'a str> {
    let candidate = latest.get(&row.prefab_id)?;
    let (Ok(best), Ok(current)) = (
        candidate.parse::<semver::Version>(),
        row.prefab_version.parse::<semver::Version>(),
    ) else {
        return None;
    };
    (best > current).then_some(candidate.as_str())
}

/// Dimension ignorée lors du calcul d'une facette : chaque facette compte ce que donnerait le
/// choix d'une de ses valeurs, les autres filtres restant appliqués.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Except {
    None,
    Prefab,
    Node,
    State,
    Upgrade,
    Suspended,
}

fn matches(
    filter: &Filter,
    latest: &HashMap<String, String>,
    row: &ServerIndexRow,
    except: Except,
) -> bool {
    if let Some(q) = filter.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        let q = q.to_lowercase();
        let found = row.name.to_lowercase().contains(&q)
            || row.node_name.to_lowercase().contains(&q)
            || row.prefab_id.to_lowercase().contains(&q)
            || row.id.to_string().starts_with(&q);
        if !found {
            return false;
        }
    }
    if except != Except::Prefab
        && !filter.prefabs.is_empty()
        && !filter.prefabs.contains(&row.prefab_id)
    {
        return false;
    }
    if except != Except::Node && !filter.nodes.is_empty() && !filter.nodes.contains(&row.node_id) {
        return false;
    }
    if except != Except::State && !filter.states.is_empty() && !filter.states.contains(&row.state) {
        return false;
    }
    if except != Except::Upgrade
        && filter
            .upgrade
            .is_some_and(|wanted| newer(latest, row).is_some() != wanted)
    {
        return false;
    }
    if except != Except::Suspended
        && filter
            .suspended
            .is_some_and(|wanted| row.suspended != wanted)
    {
        return false;
    }
    true
}

fn compare(a: &ServerIndexRow, b: &ServerIndexRow, key: SortKey) -> Ordering {
    let by_name = || a.name.to_lowercase().cmp(&b.name.to_lowercase());
    match key {
        SortKey::Name => by_name(),
        SortKey::State => a.state.cmp(&b.state).then_with(by_name),
        SortKey::Prefab => a
            .prefab_id
            .cmp(&b.prefab_id)
            .then_with(|| a.prefab_version.cmp(&b.prefab_version))
            .then_with(by_name),
        SortKey::Node => a.node_name.cmp(&b.node_name).then_with(by_name),
        SortKey::Memory => a.memory_mb.cmp(&b.memory_mb).then_with(by_name),
        SortKey::Cpu => a.cpu_percent.cmp(&b.cpu_percent).then_with(by_name),
        SortKey::Disk => a.disk_mb.cmp(&b.disk_mb).then_with(by_name),
        SortKey::Created => a.created_at.cmp(&b.created_at).then_with(by_name),
    }
    .then_with(|| a.id.cmp(&b.id))
}

/// Compte les valeurs d'une facette parmi les serveurs retenus sans cette dimension.
fn facet(
    rows: &[ServerIndexRow],
    keep: impl Fn(&ServerIndexRow) -> bool,
    key: impl Fn(&ServerIndexRow) -> (String, String),
) -> Vec<FacetValue> {
    let mut counts: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for row in rows.iter().filter(|row| keep(row)) {
        let (value, label) = key(row);
        counts.entry(value).or_insert((label, 0)).1 += 1;
    }
    counts
        .into_iter()
        .map(|(value, (label, count))| FacetValue {
            value,
            label,
            count,
        })
        .collect()
}

pub async fn search(state: &ServersState, search: &Search) -> Result<Page, ServersError> {
    let mut conn = state.pool.acquire().await?;
    let rows = ServerRepository::index(&mut conn).await?;
    drop(conn);
    let latest = latest_versions(state).await?;
    let filter = &search.filter;
    let latest_ref = &latest;
    let keep =
        |except: Except| move |row: &ServerIndexRow| matches(filter, latest_ref, row, except);

    let facets = Facets {
        // Libellé d'un prefab : sa dernière version (son icône en dépend).
        prefab: facet(&rows, keep(Except::Prefab), |r| {
            let version = latest_ref
                .get(&r.prefab_id)
                .cloned()
                .unwrap_or_else(|| r.prefab_version.clone());
            (r.prefab_id.clone(), version)
        }),
        node: facet(&rows, keep(Except::Node), |r| {
            (r.node_id.to_string(), r.node_name.clone())
        }),
        state: facet(&rows, keep(Except::State), |r| {
            (r.state.clone(), r.state.clone())
        }),
        upgrade: rows
            .iter()
            .filter(|row| keep(Except::Upgrade)(row) && newer(&latest, row).is_some())
            .count(),
        suspended: rows
            .iter()
            .filter(|row| keep(Except::Suspended)(row) && row.suspended)
            .count(),
    };

    let mut selected: Vec<&ServerIndexRow> =
        rows.iter().filter(|row| keep(Except::None)(row)).collect();
    selected.sort_by(|a, b| {
        let order = compare(a, b, search.sort);
        if search.descending {
            order.reverse()
        } else {
            order
        }
    });
    let total = selected.len();
    let per_page = search.per_page.clamp(1, MAX_PER_PAGE);
    let start = search.page.saturating_sub(1).saturating_mul(per_page);
    let page_rows: Vec<&ServerIndexRow> = selected
        .iter()
        .skip(start)
        .take(per_page)
        .copied()
        .collect();

    let ids: Vec<Uuid> = page_rows.iter().map(|row| row.id).collect();
    let latest_of_page: HashMap<Uuid, String> = page_rows
        .iter()
        .filter_map(|row| newer(&latest, row).map(|v| (row.id, v.to_owned())))
        .collect();
    let mut conn = state.pool.acquire().await?;
    let mut records: HashMap<Uuid, _> = ServerRepository::by_ids(&mut conn, &ids)
        .await?
        .into_iter()
        .map(|record| (record.id, record))
        .collect();
    // Dans l'ordre de la page ; un serveur supprimé entre les deux lectures en est absent.
    let ordered: Vec<_> = ids.iter().filter_map(|id| records.remove(id)).collect();
    let items = super::views_of(&mut conn, ordered).await?;

    Ok(Page {
        items,
        total,
        facets,
        latest: latest_of_page,
        ids: search
            .all_ids
            .then(|| selected.iter().take(MAX_IDS).map(|row| row.id).collect()),
    })
}

#[cfg(test)]
mod tests {
    use time::OffsetDateTime;

    use super::*;

    fn row(name: &str, prefab: &str, version: &str, state: &str) -> ServerIndexRow {
        ServerIndexRow {
            id: Uuid::now_v7(),
            name: name.to_owned(),
            node_id: Uuid::nil(),
            node_name: "dev-1".to_owned(),
            prefab_id: prefab.to_owned(),
            prefab_version: version.to_owned(),
            state: state.to_owned(),
            suspended: false,
            memory_mb: 2048,
            cpu_percent: 200,
            disk_mb: 0,
            created_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn upgrade_compares_semver_not_text() {
        let latest = HashMap::from([("mc".to_owned(), "1.10.0".to_owned())]);
        assert_eq!(
            newer(&latest, &row("a", "mc", "1.9.0", "running")),
            Some("1.10.0")
        );
        assert_eq!(newer(&latest, &row("b", "mc", "1.10.0", "running")), None);
        assert_eq!(newer(&latest, &row("c", "other", "0.1.0", "running")), None);
    }

    #[test]
    fn a_facet_ignores_its_own_filter_but_keeps_the_others() {
        let latest = HashMap::new();
        let rows = vec![
            row("a", "mc", "1.0.0", "running"),
            row("b", "mc", "1.0.0", "offline"),
            row("c", "terraria", "1.0.0", "running"),
        ];
        let filter = Filter {
            prefabs: vec!["mc".to_owned()],
            states: vec!["running".to_owned()],
            ..Filter::default()
        };
        let prefab = facet(
            &rows,
            |r| matches(&filter, &latest, r, Except::Prefab),
            |r| (r.prefab_id.clone(), r.prefab_id.clone()),
        );
        // Les prefabs comptés parmi les serveurs en marche, quel que soit le prefab choisi.
        assert_eq!(
            prefab
                .iter()
                .map(|f| (f.value.as_str(), f.count))
                .collect::<Vec<_>>(),
            [("mc", 1), ("terraria", 1)]
        );
    }
}
