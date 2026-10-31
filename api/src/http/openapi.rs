//! Document OpenAPI (JSON uniquement), assemblé à partir des fragments de chaque domaine.

use crate::http::extract::Json;
use utoipa::OpenApi;

use crate::{
    audit, content, dns, enrollment, iam, nodes, oauth, prefabs, servers, settings, studio, system,
};

#[derive(OpenApi)]
#[openapi(info(title = "Kuro API", description = "API REST de l'orchestrateur Kuro."))]
struct ApiDoc;

/// Document complet : socle + fragments des domaines.
pub fn document() -> utoipa::openapi::OpenApi {
    let mut doc = ApiDoc::openapi();
    doc.merge(system::http::SystemApi::openapi());
    doc.merge(crate::setup::http::SetupApi::openapi());
    doc.merge(iam::http::IamApi::openapi());
    doc.merge(audit::http::AuditApi::openapi());
    doc.merge(nodes::http::NodesApi::openapi());
    doc.merge(enrollment::http::EnrollmentApi::openapi());
    doc.merge(prefabs::http::PrefabsApi::openapi());
    doc.merge(studio::http::StudioApi::openapi());
    doc.merge(servers::http::ServersApi::openapi());
    doc.merge(settings::http::SettingsApi::openapi());
    doc.merge(oauth::http::OAuthApi::openapi());
    doc.merge(dns::http::DnsApi::openapi());
    doc.merge(content::http::ContentSourcesApi::openapi());
    doc.merge(crate::notifications::http::NotificationsApi::openapi());
    doc
}

pub fn to_json() -> Result<String, serde_json::Error> {
    document().to_pretty_json()
}

pub async fn serve() -> Json<utoipa::openapi::OpenApi> {
    Json(document())
}
