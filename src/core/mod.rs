pub mod blacklist;
pub mod commands;
pub mod events;
pub mod state;
pub mod types;

pub use blacklist::BlacklistEngine;
pub use commands::CoreCommand;
pub use events::CoreEvent;
pub use state::AppState;
pub use types::*;

use tokio::sync::{broadcast, mpsc};

pub struct CoreChannels {
    pub command_sender: mpsc::Sender<CoreCommand>,
    pub command_receiver: mpsc::Receiver<CoreCommand>,
    pub event_sender: broadcast::Sender<CoreEvent>,
}

impl CoreChannels {
    pub fn new(command_buffer_size: usize, event_buffer_size: usize) -> Self {
        let (command_sender, command_receiver) = mpsc::channel(command_buffer_size);
        let (event_sender, _) = broadcast::channel(event_buffer_size);

        Self {
            command_sender,
            command_receiver,
            event_sender,
        }
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<CoreEvent> {
        self.event_sender.subscribe()
    }
}
