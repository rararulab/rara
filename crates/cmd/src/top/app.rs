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

use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};

use crate::top::{
    client::KernelClient,
    types::{
        AgentInfo, AgentTimeline, ApprovalRequest, AuditEvent, KernelEventEnvelope,
        MetricsSnapshot, PanelFocus, SessionState, SessionStats, SessionView, SystemStats,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Sessions,
    Agents,
    Approvals,
    Audit,
    SessionDetails,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Sessions,
        Tab::Agents,
        Tab::Approvals,
        Tab::Audit,
        Tab::SessionDetails,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Sessions => "Sessions",
            Tab::Agents => "Agents",
            Tab::Approvals => "Approvals",
            Tab::Audit => "Audit",
            Tab::SessionDetails => "Details",
        }
    }
}

pub struct App {
    pub tab:           Tab,
    pub scroll_offset: usize,
    pub stats:         Option<SystemStats>,
    pub sessions_list: Vec<SessionStats>,
    pub agents:        Vec<AgentInfo>,
    pub approvals:     Vec<ApprovalRequest>,
    pub audit:         Vec<AuditEvent>,
    pub session_state: SessionState,
    pub connected:     bool,
    pub error:         Option<String>,
    pub should_quit:   bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            tab:           Tab::Sessions,
            scroll_offset: 0,
            stats:         None,
            sessions_list: Vec::new(),
            agents:        Vec::new(),
            approvals:     Vec::new(),
            audit:         Vec::new(),
            session_state: SessionState::new(),
            connected:     false,
            error:         None,
            should_quit:   false,
        }
    }

    /// Returns the number of rows in the current tab's data.
    pub fn current_row_count(&self) -> usize {
        match self.tab {
            Tab::Sessions => self.sessions_list.len(),
            Tab::Agents => self.agents.len(),
            Tab::Approvals => self.approvals.len(),
            Tab::Audit => self.audit.len(),
            Tab::SessionDetails => self.session_state.sessions.len(),
        }
    }

    fn handle_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.should_quit = true;
            }
            KeyCode::Char('1') => {
                self.tab = Tab::Sessions;
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
            KeyCode::Char('5') => {
                self.tab = Tab::SessionDetails;
                self.scroll_offset = 0;
            }
            KeyCode::Tab => {
                if self.tab == Tab::SessionDetails {
                    self.session_state.focus = match self.session_state.focus {
                        PanelFocus::SessionList => PanelFocus::Gantt,
                        PanelFocus::Gantt => PanelFocus::SessionTree,
                        PanelFocus::SessionTree => PanelFocus::SessionList,
                    };
                }
            }
            KeyCode::BackTab => {
                if self.tab == Tab::SessionDetails {
                    self.session_state.focus = match self.session_state.focus {
                        PanelFocus::SessionList => PanelFocus::SessionTree,
                        PanelFocus::Gantt => PanelFocus::SessionList,
                        PanelFocus::SessionTree => PanelFocus::Gantt,
                    };
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.tab == Tab::SessionDetails {
                    self.handle_sessions_up();
                } else {
                    self.scroll_offset = self.scroll_offset.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.tab == Tab::SessionDetails {
                    self.handle_sessions_down();
                } else {
                    let max = self.current_row_count().saturating_sub(1);
                    if self.scroll_offset < max {
                        self.scroll_offset += 1;
                    }
                }
            }
            KeyCode::Enter => {
                if self.tab == Tab::SessionDetails
                    && self.session_state.focus == PanelFocus::SessionList
                    && !self.session_state.sessions.is_empty()
                {
                    // Jump focus to Gantt panel on Enter from session list
                    self.session_state.focus = PanelFocus::Gantt;
                    self.session_state.gantt_selected = 0;
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                // refresh is handled externally; this is just a signal
            }
            _ => {}
        }
    }

    fn handle_sessions_up(&mut self) {
        let ss = &mut self.session_state;
        match ss.focus {
            PanelFocus::SessionList => {
                ss.selected_session = ss.selected_session.saturating_sub(1);
                // Reset sub-panel selections when switching session
                ss.gantt_selected = 0;
                ss.tree_selected = 0;
            }
            PanelFocus::Gantt => {
                ss.gantt_selected = ss.gantt_selected.saturating_sub(1);
            }
            PanelFocus::SessionTree => {
                ss.tree_selected = ss.tree_selected.saturating_sub(1);
            }
        }
    }

    fn handle_sessions_down(&mut self) {
        let ss = &mut self.session_state;
        match ss.focus {
            PanelFocus::SessionList => {
                let max = ss.sessions.len().saturating_sub(1);
                if ss.selected_session < max {
                    ss.selected_session += 1;
                    // Reset sub-panel selections when switching session
                    ss.gantt_selected = 0;
                    ss.tree_selected = 0;
                }
            }
            PanelFocus::Gantt => {
                if let Some(sv) = ss.sessions.get_index(ss.selected_session).map(|(_, v)| v) {
                    let max = sv.agents.len().saturating_sub(1);
                    if ss.gantt_selected < max {
                        ss.gantt_selected += 1;
                    }
                }
            }
            PanelFocus::SessionTree => {
                if let Some(sv) = ss.sessions.get_index(ss.selected_session).map(|(_, v)| v) {
                    let max = sv.agents.len().saturating_sub(1);
                    if ss.tree_selected < max {
                        ss.tree_selected += 1;
                    }
                }
            }
        }
    }

    pub fn push_kernel_event(&mut self, event: KernelEventEnvelope) {
        let now = Instant::now();

        // Determine which session this event belongs to.
        let session_id = self.resolve_session_id(&event);

        let ss = &mut self.session_state;

        // Get or create the session.
        let session_view = ss
            .sessions
            .entry(session_id.clone())
            .or_insert_with(|| SessionView {
                session_id: session_id.clone(),
                agents:     indexmap::IndexMap::new(),
                first_seen: now,
                last_event: now,
            });

        session_view.last_event = now;

        // If the event has an agent_id, update the agent timeline.
        if let Some(ref agent_id) = event.common.agent_id {
            let timeline = session_view
                .agents
                .entry(agent_id.clone())
                .or_insert_with(|| AgentTimeline {
                    agent_id:  agent_id.clone(),
                    name:      agent_id.clone(),
                    parent_id: None,
                    start:     now,
                    end:       None,
                    state:     "Unknown".to_string(),
                    metrics:   MetricsSnapshot::default(),
                    events:    Vec::new(),
                });

            // Cap events per agent to avoid unbounded growth.
            const MAX_EVENTS_PER_AGENT: usize = 100;
            if timeline.events.len() >= MAX_EVENTS_PER_AGENT {
                timeline
                    .events
                    .drain(..=(timeline.events.len() - MAX_EVENTS_PER_AGENT));
            }
            timeline.events.push(event);
        }

        // Sort sessions by last_event descending.
        ss.sessions
            .sort_by(|_k1, v1, _k2, v2| v2.last_event.cmp(&v1.last_event));
    }

    /// Resolve a session_id for the given event.
    fn resolve_session_id(&self, event: &KernelEventEnvelope) -> String {
        // 1. Explicit session_id in common fields.
        if let Some(ref sid) = event.common.session_id {
            if !sid.is_empty() {
                return sid.clone();
            }
        }

        // 2. Look up agent_id in existing sessions.
        if let Some(ref agent_id) = event.common.agent_id {
            for (session_id, sv) in &self.session_state.sessions {
                if sv.agents.contains_key(agent_id) {
                    return session_id.clone();
                }
            }

            // 3. Look up in sessions list data.
            for p in &self.sessions_list {
                if p.agent_id == *agent_id {
                    return p.session_id.clone();
                }
            }
        }

        // Fallback.
        "unknown".to_string()
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
        let (sessions, agents, approvals, audit) = tokio::join!(
            client.sessions(),
            client.agents(),
            client.approvals(),
            client.audit(50),
        );

        if let Ok(s) = sessions {
            self.sessions_list = s;
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

        // Update session state from sessions data.
        self.sync_session_details();
    }

    /// Sync SessionState with session data obtained from the kernel API.
    fn sync_session_details(&mut self) {
        let now = Instant::now();
        for p in &self.sessions_list {
            let ss = &mut self.session_state;

            // Use uptime_ms to compute the actual start time of this process.
            let agent_start = now.checked_sub(Duration::from_millis(p.uptime_ms)).unwrap();

            let session_view =
                ss.sessions
                    .entry(p.session_id.clone())
                    .or_insert_with(|| SessionView {
                        session_id: p.session_id.clone(),
                        agents:     indexmap::IndexMap::new(),
                        first_seen: agent_start,
                        last_event: now,
                    });

            // Update session first_seen if this agent started earlier.
            if agent_start < session_view.first_seen {
                session_view.first_seen = agent_start;
            }
            session_view.last_event = now;

            let timeline = session_view
                .agents
                .entry(p.agent_id.clone())
                .or_insert_with(|| AgentTimeline {
                    agent_id:  p.agent_id.clone(),
                    name:      p.name.clone(),
                    parent_id: p.parent_id.clone(),
                    start:     agent_start,
                    end:       None,
                    state:     p.state.clone(),
                    metrics:   p.metrics.clone(),
                    events:    Vec::new(),
                });

            // Always update mutable fields from the latest process data.
            timeline.name = p.name.clone();
            timeline.parent_id = p.parent_id.clone();
            timeline.state = p.state.clone();
            timeline.metrics = p.metrics.clone();
            timeline.start = agent_start;

            // Mark completed/failed agents.
            match p.state.as_str() {
                "Completed" | "completed" | "Failed" | "failed" => {
                    if timeline.end.is_none() {
                        timeline.end = Some(now);
                    }
                }
                _ => {
                    timeline.end = None;
                }
            }
        }

        // Sort sessions by last_event descending.
        self.session_state
            .sessions
            .sort_by(|_k1, v1, _k2, v2| v2.last_event.cmp(&v1.last_event));
    }

    /// Run the main TUI event loop.
    pub async fn run(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        client: &KernelClient,
    ) -> std::io::Result<()> {
        let mut poll_interval = tokio::time::interval(Duration::from_secs(1));
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let stream_client = client.clone();
        let stream_task = tokio::spawn(async move {
            stream_client.stream_events(event_tx).await;
        });
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
                maybe_kernel_event = event_rx.recv() => {
                    if let Some(event) = maybe_kernel_event {
                        self.push_kernel_event(event);
                    }
                }
                maybe_event = poll_crossterm_event() => {
                    if let Some(Event::Key(key)) = maybe_event {
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

            if self.should_quit {
                break;
            }
        }

        stream_task.abort();

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
