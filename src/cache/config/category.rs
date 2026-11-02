//! Category config (server-only, names only). Unknown
//! opcodes are skipped without consuming an operand.

use super::ConfigType;
use crate::io::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct CategoryType {
    pub id: usize,
    pub name: Option<String>,
}

impl ConfigType for CategoryType {
    fn new(id: usize) -> Self {
        CategoryType { id, name: None }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        if code == 1 {
            self.name = Some(dat.gjstr(10));
        }
        Ok(())
    }

    fn debug_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl std::fmt::Display for CategoryType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match &self.name {
            Some(n) => f.write_str(n),
            None => write!(f, "category_{}", self.id),
        }
    }
}
