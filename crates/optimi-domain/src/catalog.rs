use crate::id::{ActionId, ProfileId};
use crate::model::{
    Evidence, MeasurementStatus, Risk, ServiceRunState, ServiceStartType, ServiceState,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogAction {
    pub id: ActionId,
    pub service: String,
    pub risk: Risk,
    pub target: ServiceState,
    pub evidence: Evidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub id: ProfileId,
    pub action_ids: Vec<ActionId>,
}

#[derive(Debug, Clone)]
pub struct Catalog {
    pub version: &'static str,
    actions: Vec<CatalogAction>,
    profiles: Vec<Profile>,
}

impl Catalog {
    pub fn embedded() -> Self {
        let action = CatalogAction {
            id: ActionId::parse("diagtrack-privacy").expect("embedded action id is valid"),
            service: "DiagTrack".to_owned(),
            risk: Risk::Low,
            target: ServiceState {
                run_state: ServiceRunState::Stopped,
                start_type: ServiceStartType::Disabled,
            },
            evidence: Evidence {
                source: "Microsoft Learn: Configure Windows diagnostic data in your organization".to_owned(),
                url: "https://learn.microsoft.com/en-us/windows/privacy/configure-windows-diagnostic-data-in-your-organization"
                    .to_owned(),
                date: "2025-09-11".to_owned(),
                measurement: MeasurementStatus::NotMeasured,
                claim_key: "claim.diagtrack-privacy".to_owned(),
            },
        };
        let profile = Profile {
            id: ProfileId::parse("competitive").expect("embedded profile id is valid"),
            action_ids: vec![action.id.clone()],
        };
        Self {
            version: "0.1",
            actions: vec![action],
            profiles: vec![profile],
        }
    }

    pub fn action(&self, id: &ActionId) -> Option<&CatalogAction> {
        self.actions.iter().find(|action| &action.id == id)
    }

    pub fn profile(&self, id: &ProfileId) -> Option<&Profile> {
        self.profiles.iter().find(|profile| &profile.id == id)
    }
}
