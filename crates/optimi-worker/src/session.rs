use std::path::Path;

use optimi_ipc::identity::ClientIdentity;

#[derive(Debug, Clone)]
pub struct SessionPolicy {
    executable: String,
    started_at: u64,
}

#[derive(Debug, Clone)]
pub struct Session {
    client: ClientIdentity,
}

impl SessionPolicy {
    pub fn new(executable: impl AsRef<Path>, started_at: u64) -> Self {
        Self {
            executable: executable.as_ref().to_string_lossy().into_owned(),
            started_at,
        }
    }

    pub fn authorize(&self, client: ClientIdentity) -> Option<Session> {
        if client.process.creation_time >= self.started_at {
            return None;
        }
        if !client
            .image
            .to_string_lossy()
            .eq_ignore_ascii_case(&self.executable)
        {
            return None;
        }
        Some(Session { client })
    }
}

impl Session {
    pub fn allows(&self, client: &ClientIdentity) -> bool {
        client == &self.client
    }

    pub fn process_id(&self) -> u32 {
        self.client.process.process_id
    }

    pub fn user_sid(&self) -> &str {
        &self.client.user_sid
    }
}
