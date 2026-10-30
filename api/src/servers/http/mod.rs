//! Routes HTTP de l'espace admin pour le domaine `servers`, montées sous `/api/admin/v1`.

mod backups;
mod console;
mod content;
mod error;
mod files;
mod loaders;
mod schedules;
mod search;
mod servers;
mod steam_account;
mod upgrades;

use axum::Router;
use axum::extract::FromRef;
use axum::middleware::from_fn;
use axum::routing::{get, patch, post, put};
use utoipa::OpenApi;

use crate::iam::application::IamState;
use crate::servers::application::ServersState;

#[derive(OpenApi)]
#[openapi(
    paths(
        servers::list, servers::get, servers::create, servers::update_settings,
        servers::update_resources, servers::suspend, servers::unsuspend, servers::power, servers::reinstall, servers::delete, console::ticket, console::command,
        files::list, files::read, files::write, files::mkdir, files::rename, files::delete,
        backups::list, backups::create, backups::update, backups::restore, backups::delete,
        backups::download_ticket, backups::download, content::slots, content::list,
        content::search, content::categories, content::project, content::versions, content::install, content::update, content::toggle,
        content::set_values,
        content::remove, content::upload, content::modpack_search, content::modpack_versions,
        content::install_modpack, files::upload, loaders::list, schedules::list,
        schedules::create, schedules::get, schedules::update, schedules::delete, schedules::run,
        schedules::runs, schedules::preview, upgrades::available, upgrades::preview,
        upgrades::start, upgrades::candidates, upgrades::start_many, search::search,
        search::bulk_action, steam_account::get, steam_account::put, steam_account::delete,
    ),
    tags((name = "admin-servers", description = "Serveurs déployés à partir de prefabs"))
)]
pub struct ServersApi;

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    IamState: FromRef<S>,
    ServersState: FromRef<S>,
{
    Router::new()
        .route("/servers", get(servers::list).post(servers::create))
        .route("/servers/search", get(search::search))
        .route("/servers/bulk", post(search::bulk_action))
        .route("/content/loaders", get(loaders::list))
        .route(
            "/settings/steam-account",
            get(steam_account::get)
                .put(steam_account::put)
                .delete(steam_account::delete),
        )
        .route("/servers/{id}", get(servers::get).delete(servers::delete))
        .route("/servers/{id}/settings", patch(servers::update_settings))
        .route("/servers/{id}/resources", patch(servers::update_resources))
        .route("/servers/{id}/suspend", post(servers::suspend))
        .route("/servers/{id}/unsuspend", post(servers::unsuspend))
        .route("/servers/{id}/power", post(servers::power))
        .route("/servers/{id}/reinstall", post(servers::reinstall))
        .route("/servers/{id}/upgrades", get(upgrades::available))
        .route(
            "/servers/{id}/upgrade",
            get(upgrades::preview).post(upgrades::start),
        )
        .route("/servers/upgrade", post(upgrades::start_many))
        .route(
            "/prefabs/{id}/{version}/upgrades",
            get(upgrades::candidates),
        )
        .route("/servers/{id}/console/ticket", post(console::ticket))
        .route("/servers/{id}/console", get(console::socket))
        .route("/servers/{id}/command", post(console::command))
        .route(
            "/servers/{id}/schedules",
            get(schedules::list).post(schedules::create),
        )
        .route("/servers/{id}/schedules/preview", post(schedules::preview))
        .route(
            "/servers/{id}/schedules/{schedule_id}",
            get(schedules::get)
                .put(schedules::update)
                .delete(schedules::delete),
        )
        .route(
            "/servers/{id}/schedules/{schedule_id}/run",
            post(schedules::run),
        )
        .route(
            "/servers/{id}/schedules/{schedule_id}/runs",
            get(schedules::runs),
        )
        .route(
            "/servers/{id}/files",
            get(files::list).delete(files::delete),
        )
        .route(
            "/servers/{id}/files/content",
            get(files::read).put(files::write),
        )
        .route("/servers/{id}/files/directories", post(files::mkdir))
        .route("/servers/{id}/files/rename", post(files::rename))
        // Envoi d'un fichier entier : aussi long que le téléversement, hors délai global.
        .route(
            "/servers/{id}/files/upload",
            put(files::upload).layer(from_fn(crate::http::timeout::exempt)),
        )
        .route(
            "/servers/{id}/backups",
            get(backups::list).post(backups::create),
        )
        .route(
            "/servers/{id}/backups/{backup_id}",
            patch(backups::update).delete(backups::delete),
        )
        .route(
            "/servers/{id}/backups/{backup_id}/restore",
            post(backups::restore),
        )
        .route(
            "/servers/{id}/backups/{backup_id}/download/ticket",
            post(backups::download_ticket),
        )
        .route(
            "/servers/{id}/backups/{backup_id}/download",
            post(backups::download),
        )
        .route("/servers/{id}/content", get(content::slots))
        .route(
            "/servers/{id}/content/{slot}",
            get(content::list).post(content::install),
        )
        .route("/servers/{id}/content/{slot}/search", get(content::search))
        .route(
            "/servers/{id}/content/{slot}/categories",
            get(content::categories),
        )
        .route(
            "/servers/{id}/content/{slot}/upload",
            post(content::upload).layer(from_fn(crate::http::timeout::exempt)),
        )
        // Attend le redéploiement du serveur (plusieurs minutes au plus) : hors délai global.
        .route(
            "/servers/{id}/content/{slot}/modpacks",
            post(content::install_modpack).layer(from_fn(crate::http::timeout::exempt)),
        )
        .route(
            "/servers/{id}/content/{slot}/modpacks/search",
            get(content::modpack_search),
        )
        .route(
            "/servers/{id}/content/{slot}/modpacks/{project}/versions",
            get(content::modpack_versions),
        )
        .route(
            "/servers/{id}/content/{slot}/projects/{project}",
            get(content::project),
        )
        .route(
            "/servers/{id}/content/{slot}/projects/{project}/versions",
            get(content::versions),
        )
        .route(
            "/servers/{id}/content/{slot}/{content_id}",
            patch(content::toggle).delete(content::remove),
        )
        .route(
            "/servers/{id}/content/{slot}/{content_id}/values",
            put(content::set_values),
        )
        .route(
            "/servers/{id}/content/{slot}/{content_id}/update",
            post(content::update),
        )
}
