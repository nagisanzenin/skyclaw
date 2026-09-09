//! Resolve outbound tools from authenticated session transport, never a
//! model-supplied channel argument or an arbitrary fallback for unknown input.
use std::{collections::HashMap, sync::Arc};
use temm1e_core::{types::error::Temm1eError, Channel};

pub(crate) enum ChannelTarget {
    Single(Arc<dyn Channel>),
    Routed {
        channels: HashMap<String, Arc<dyn Channel>>,
        heartbeat: Option<Arc<dyn Channel>>,
    },
}

impl ChannelTarget {
    pub fn resolve(&self, channel: &str) -> Result<&dyn Channel, Temm1eError> {
        match self {
            Self::Single(target) => Ok(target.as_ref()),
            Self::Routed {
                channels,
                heartbeat,
            } => {
                let target = if channel == "heartbeat" {
                    heartbeat.as_ref()
                } else {
                    channels.get(channel)
                };
                target.map(|target| target.as_ref()).ok_or_else(|| {
                    Temm1eError::PermissionDenied("No configured transport for this session".into())
                })
            }
        }
    }
}
