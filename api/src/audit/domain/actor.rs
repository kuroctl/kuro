use uuid::Uuid;

/// Qui agit. `Customer` est réservé à l'espace client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorKind {
    Admin,
    Customer,
    Node,
    Cli,
    System,
    Anonymous,
    /// Agent IA (MCP, ADR-0023) agissant au nom de l'admin identifié.
    Agent,
    /// Propriétaire d'un serveur (ADR-0051), identifié par une extension : son identifiant
    /// opaque est dans `details.owner_ref`, pas dans `actor_id`.
    Owner,
}

impl ActorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Customer => "customer",
            Self::Node => "node",
            Self::Cli => "cli",
            Self::System => "system",
            Self::Anonymous => "anonymous",
            Self::Agent => "agent",
            Self::Owner => "owner",
        }
    }
}

/// Acteur identifié par son identifiant uniquement (minimisation RGPD : jamais d'e-mail ici).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actor {
    pub kind: ActorKind,
    pub id: Option<Uuid>,
}

impl Actor {
    pub fn admin(id: Uuid) -> Self {
        Self {
            kind: ActorKind::Admin,
            id: Some(id),
        }
    }

    /// Propriétaire d'un serveur (ADR-0051) : sans uuid, voir [`AuditContext::as_owner`].
    pub fn owner() -> Self {
        Self {
            kind: ActorKind::Owner,
            id: None,
        }
    }

    /// Agent IA, identifié par l'admin au nom de qui il agit.
    pub fn agent(user_id: Uuid) -> Self {
        Self {
            kind: ActorKind::Agent,
            id: Some(user_id),
        }
    }

    pub fn anonymous() -> Self {
        Self {
            kind: ActorKind::Anonymous,
            id: None,
        }
    }
}

/// Agent IA à l'origine d'une requête (MCP, ADR-0023) : recopié dans chaque événement
/// (`details.agent`), pour savoir quelle application a agi et par quel accès.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRef {
    pub client_id: String,
    pub grant_id: Uuid,
}

/// Acteur et contexte de la requête, connus de toute la couche application.
#[derive(Debug, Clone)]
pub struct AuditContext {
    pub actor: Actor,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    /// Identifiant de requête, partagé avec les logs techniques.
    pub request_id: Option<String>,
    /// Agent IA, quand l'acteur est `agent`.
    pub agent: Option<AgentRef>,
    /// Propriétaire (ADR-0051), quand l'acteur est `owner` : recopié dans `details.owner_ref`.
    pub owner: Option<String>,
}

impl AuditContext {
    /// Commande d'administration lancée sur l'hôte : l'agent indique l'utilisateur système.
    pub fn cli(command: &str) -> Self {
        let os_user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "unknown".to_owned());
        Self {
            actor: Actor {
                kind: ActorKind::Cli,
                id: None,
            },
            ip: None,
            user_agent: Some(format!("kuro-api {command} (os user: {os_user})")),
            request_id: None,
            agent: None,
            owner: None,
        }
    }

    pub fn system() -> Self {
        Self {
            actor: Actor {
                kind: ActorKind::System,
                id: None,
            },
            ip: None,
            user_agent: None,
            request_id: None,
            agent: None,
            owner: None,
        }
    }

    /// Action d'une tâche planifiée (ADR-0022) : acteur système, identifié par la tâche.
    pub fn scheduler(schedule_id: Uuid) -> Self {
        Self {
            actor: Actor {
                kind: ActorKind::System,
                id: Some(schedule_id),
            },
            ..Self::system()
        }
    }

    /// Même requête, faite par le propriétaire `owner_ref` d'un serveur (ADR-0051).
    pub fn as_owner(&self, owner_ref: &str) -> Self {
        Self {
            actor: Actor::owner(),
            owner: Some(owner_ref.to_owned()),
            ..self.clone()
        }
    }

    /// Même requête, faite par un agent IA au nom de l'admin `user_id`.
    pub fn as_agent(&self, user_id: Uuid, agent: AgentRef) -> Self {
        Self {
            actor: Actor::agent(user_id),
            agent: Some(agent),
            ..self.clone()
        }
    }

    /// Même requête, acteur identifié (après authentification).
    pub fn with_actor(&self, actor: Actor) -> Self {
        Self {
            actor,
            ..self.clone()
        }
    }
}
