// Copyright 2025 Rararulab
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};

use crate::top::{
    client::KernelClient,
    types::{AgentInfo, ApprovalRequest, AuditEvent, ProcessStats, SystemStats},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Processes,
    Agents,
    Approvals,
    Audit,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Processes, Tab::Agents, Tab::Approvals, Tab::Audit];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Processes => "Processes",
            Tab::Agents => "Agents",
            Tab::Approvals => "Approvals",
            Tab::Audit => "Audit",
        }
    }
}

pub struct App {
    pub tab:           Tab,
    pub scroll_offset: usize,
    pub stats:         Option<SystemStats>,
    pub processes:     Vec<ProcessStats>,
    pub agents:        Vec<AgentInfo>,
    pub approvals:     Vec<ApprovalRequest>,
    pub audit:         Vec<AuditEvent>,
    pub connected:     bool,
    pub error:         Option<String>,
    pub should_quit:   bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            tab:           Tab::Processes,
            scroll_offset: 0,
            stats:         None,
            processes:     Vec::new(),
            agents:        Vec::new(),
            approvals:     Vec::new(),
            audit:         Vec::new(),
            connected:     false,
            error:         None,
            should_quit:   false,
        }
    }

    /// Returns the number of rows in the current tab's data.
    pub fn current_row_count(&self) -> usize {
        match self.tab {
            Tab::Processes => self.processes.len(),
            Tab::Agents => self.agents.len(),
            Tab::Approvals => self.approvals.len(),
            Tab::Audit => self.audit.len(),
        }
    }

    fn handle_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.should_quit = true;
            }
            KeyCode::Char('1') => {
                self.tab = Tab::Processes;
                self.scroll_offset = 0;
            }
            KeyCode::Char('2') => {
                self.tab = Tab::Agents;
                self.scroll_offset = 0;
            }
            KeyCode::Char('3') => {
                self.tab = Tab::Approvals;
                self.scroll_offset = 0;
            }
            KeyCode::Char('4') => {
                self.tab = Tab::Audit;
                self.scroll_offset = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let max = self.current_row_count().saturating_sub(1);
                if self.scroll_offset < max {
                    self.scroll_offset += 1;
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                // refresh is handled externally; this is just a signal
            }
            _ => {}
        }
    }

    /// Fetch all data from the kernel API.
    pub async fn refresh(&mut self, client: &KernelClient) {
        // We fetch stats first; if it fails, mark as disconnected.
        match client.stats().await {
            Ok(s) => {
                self.stats = Some(s);
                self.connected = true;
                self.error = None;
            }
            Err(e) => {
                self.connected = false;
                self.error = Some(format!("{e}"));
                return;
            }
        }

        // Fetch the rest in parallel; individual failures are tolerated.
        let (processes, agents, approvals, audit) = tokio::join!(
            client.processes(),
            client.agents(),
            client.approvals(),
            client.audit(50),
        );

        if let Ok(p) = processes {
            self.processes = p;
        }
        if let Ok(a) = agents {
            self.agents = a;
        }
        if let Ok(a) = approvals {
            self.approvals = a;
        }
        if let Ok(a) = audit {
            self.audit = a;
        }
    }

    /// Run the main TUI event loop.
    pub async fn run(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        client: &KernelClient,
    ) -> std::io::Result<()> {
        let mut poll_interval = tokio::time::interval(Duration::from_secs(1));
        // Do an initial fetch immediately.
        self.refresh(client).await;

        loop {
            // Draw the UI.
            terminal.draw(|frame| crate::top::ui::render(frame, self))?;

            // Wait for either a key event or the poll interval.
            tokio::select! {
                _ = poll_interval.tick() => {
                    self.refresh(client).await;
                }
                maybe_event = poll_crossterm_event() => {
                    if let Some(ev) = maybe_event {
                        if let Event::Key(key) = ev {
                            if key.kind == KeyEventKind::Press {
                                // 'r' triggers immediate refresh
                                if key.code == KeyCode::Char('r') || key.code == KeyCode::Char('R')
                                {
                                    self.refresh(client).await;
                                }
                                self.handle_key(key.code);
                            }
                        }
                    }
                }
            }

            if self.should_quit {
                break;
            }
        }

        Ok(())
    }
}

/// Non-blocking crossterm event poll that yields to tokio.
async fn poll_crossterm_event() -> Option<Event> {
    // We use tokio::task::spawn_blocking to avoid blocking the async runtime.
    // crossterm::event::poll + read are blocking operations.
    tokio::task::spawn_blocking(|| {
        if event::poll(Duration::from_millis(100)).unwrap_or(false) {
            event::read().ok()
        } else {
            None
        }
    })
    .await
    .ok()
    .flatten()
}
