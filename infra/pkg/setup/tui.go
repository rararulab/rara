/*
 * Copyright 2025 Rararulab
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package setup

import (
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/charmbracelet/bubbles/spinner"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/huh"
	"github.com/charmbracelet/lipgloss"
)

// RunTUI runs the interactive TUI: first the config form, then the install progress model.
func RunTUI(ctx context.Context, initial Config) error {
	cfg, confirmed, err := runConfigForm(initial)
	if err != nil {
		return fmt.Errorf("config form: %w", err)
	}
	if !confirmed {
		fmt.Println("Aborted.")
		return nil
	}
	return runInstallModel(ctx, cfg)
}

// runConfigForm shows the huh form and returns (finalConfig, confirmed, error).
func runConfigForm(initial Config) (Config, bool, error) {
	cfg := initial
	confirmed := true

	form := huh.NewForm(
		huh.NewGroup(
			huh.NewNote().Title("rara local setup").Description("Configure your local environment"),
			huh.NewInput().Title("Cluster").Value(&cfg.ClusterName).Placeholder("rara"),
			huh.NewInput().Title("Namespace").Value(&cfg.Namespace).Placeholder("rara"),
			huh.NewInput().Title("Domain").Value(&cfg.Domain).Placeholder("rara.local"),
		),
		huh.NewGroup(
			huh.NewNote().Title("Credentials"),
			huh.NewInput().Title("PG Password").Value(&cfg.PostgresPassword).EchoMode(huh.EchoModePassword),
			huh.NewInput().Title("MinIO Password").Value(&cfg.MinioPassword).EchoMode(huh.EchoModePassword),
		),
		huh.NewGroup(
			huh.NewNote().Title("Services"),
			huh.NewConfirm().Title("Enable Ollama").Value(&cfg.EnableOllama),
			huh.NewConfirm().Title("Enable Mem0").Value(&cfg.EnableMem0),
			huh.NewConfirm().Title("Enable Memos").Value(&cfg.EnableMemos),
			huh.NewConfirm().Title("Enable Hindsight").Value(&cfg.EnableHindsight),
		),
		huh.NewGroup(
			huh.NewConfirm().Title("Deploy?").Value(&confirmed),
		),
	)

	if err := form.Run(); err != nil {
		return cfg, false, err
	}
	return cfg, confirmed, nil
}

// --- Bubble Tea install model ---

type stepStatus int

const (
	stepPending stepStatus = iota
	stepRunning
	stepDone
	stepError
)

type stepState struct {
	name    string
	status  stepStatus
	elapsed time.Duration
	startAt time.Time
	logs    []string // all EventInfo lines for this step
}

type installModel struct {
	steps     []stepState
	current   int  // index of currently running step
	cursor    int  // highlighted step (keyboard navigation)
	following bool // cursor auto-tracks current step when true
	expanded  int  // index of expanded step (-1 = none)
	spinner   spinner.Model
	start     time.Time
	elapsed   time.Duration
	done      bool
	finalErr  error
	warns     []string // global warnings (max 3)
	width     int
}

// tea.Msg types
type progressMsg ProgressEvent
type tickMsg time.Time

var (
	stepDoneStyle   = lipgloss.NewStyle().Foreground(lipgloss.Color("42"))
	stepRunStyle    = lipgloss.NewStyle().Foreground(lipgloss.Color("33"))
	stepPendStyle   = lipgloss.NewStyle().Foreground(lipgloss.Color("240"))
	stepErrStyle    = lipgloss.NewStyle().Foreground(lipgloss.Color("160"))
	titleStyle      = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("205"))
	elapsedStyle    = lipgloss.NewStyle().Foreground(lipgloss.Color("240"))
	activityStyle   = lipgloss.NewStyle().Foreground(lipgloss.Color("245"))
	warnStyle       = lipgloss.NewStyle().Foreground(lipgloss.Color("214"))
	cursorStyle     = lipgloss.NewStyle().Foreground(lipgloss.Color("205")).Bold(true)
	logLineStyle    = lipgloss.NewStyle().Foreground(lipgloss.Color("246"))
	hintStyle       = lipgloss.NewStyle().Foreground(lipgloss.Color("238"))
)

func newInstallModel() installModel {
	s := spinner.New(spinner.WithSpinner(spinner.Dot))
	s.Style = stepRunStyle
	return installModel{
		spinner:   s,
		start:     time.Now(),
		steps:     []stepState{},
		cursor:    0,
		following: true,
		expanded:  -1,
	}
}

func (m installModel) Init() tea.Cmd {
	return tea.Batch(m.spinner.Tick, tickCmd())
}

func tickCmd() tea.Cmd {
	return tea.Tick(100*time.Millisecond, func(t time.Time) tea.Msg {
		return tickMsg(t)
	})
}

func (m installModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		switch msg.Type {
		case tea.KeyCtrlC:
			return m, tea.Quit
		case tea.KeyUp:
			m.following = false
			if m.cursor > 0 {
				m.cursor--
			}
		case tea.KeyDown:
			m.following = false
			if m.cursor < len(m.steps)-1 {
				m.cursor++
			}
		case tea.KeyEnter:
			if len(m.steps) > 0 {
				if m.expanded == m.cursor {
					m.expanded = -1
				} else {
					m.expanded = m.cursor
				}
			}
		}
		switch msg.String() {
		case "f": // re-follow current step
			m.following = true
			m.cursor = m.current
		}
	case tea.WindowSizeMsg:
		m.width = msg.Width
	case tickMsg:
		m.elapsed = time.Since(m.start)
		return m, tickCmd()
	case spinner.TickMsg:
		var cmd tea.Cmd
		m.spinner, cmd = m.spinner.Update(msg)
		return m, cmd
	case progressMsg:
		ev := ProgressEvent(msg)
		switch ev.Kind {
		case EventStepStart:
			for len(m.steps) < ev.N {
				m.steps = append(m.steps, stepState{})
			}
			m.steps[ev.N-1] = stepState{name: ev.Name, status: stepRunning, startAt: time.Now()}
			m.current = ev.N - 1
			if m.following {
				m.cursor = m.current
			}
		case EventStepDone:
			if ev.N-1 < len(m.steps) {
				m.steps[ev.N-1].status = stepDone
				m.steps[ev.N-1].elapsed = ev.Elapsed
			}
		case EventInfo:
			if m.current < len(m.steps) {
				m.steps[m.current].logs = append(m.steps[m.current].logs, ev.Name)
			}
		case EventWarn:
			m.warns = append(m.warns, ev.Name)
			if len(m.warns) > 3 {
				m.warns = m.warns[len(m.warns)-3:]
			}
		case EventDone:
			m.done = true
			return m, tea.Quit
		case EventError:
			m.finalErr = ev.Err
			m.done = true
			if m.current < len(m.steps) {
				m.steps[m.current].status = stepError
				// Auto-expand the failed step so the user sees what was happening.
				m.expanded = m.current
				m.cursor = m.current
			}
			return m, tea.Quit
		}
	}
	return m, nil
}

func (m installModel) View() string {
	var b strings.Builder

	// Title line
	title := titleStyle.Render("rara local setup")
	if m.done {
		if m.finalErr != nil {
			title += "  " + stepErrStyle.Render("— error")
		} else {
			title += "  " + stepDoneStyle.Render("— done")
		}
	} else {
		title += "  " + stepRunStyle.Render("— installing")
	}
	b.WriteString(title + "\n\n")

	for i, s := range m.steps {
		isCursor := i == m.cursor
		isExpanded := i == m.expanded

		// Cursor indicator
		cursorMark := "  "
		if isCursor {
			cursorMark = cursorStyle.Render("▶ ")
		}

		// Step icon
		var icon string
		var nameStyle lipgloss.Style
		switch s.status {
		case stepDone:
			icon = stepDoneStyle.Render("✓")
			nameStyle = stepDoneStyle
		case stepRunning:
			icon = m.spinner.View()
			nameStyle = stepRunStyle
		case stepError:
			icon = stepErrStyle.Render("✗")
			nameStyle = stepErrStyle
		default:
			icon = stepPendStyle.Render("·")
			nameStyle = stepPendStyle
		}

		// Step name + elapsed
		line := fmt.Sprintf("%s%s  %s", cursorMark, icon, nameStyle.Render(s.name))
		switch s.status {
		case stepDone:
			line += "  " + elapsedStyle.Render(s.elapsed.Round(time.Second).String())
		case stepRunning:
			line += "  " + elapsedStyle.Render(time.Since(s.startAt).Round(time.Second).String())
		}

		// Expand hint for steps with logs
		if len(s.logs) > 0 && !isExpanded {
			line += "  " + hintStyle.Render(fmt.Sprintf("[Enter ↵ %d lines]", len(s.logs)))
		}
		b.WriteString(line + "\n")

		// Activity line for currently running step (when not expanded)
		if s.status == stepRunning && i == m.current && !isExpanded && len(s.logs) > 0 {
			latest := s.logs[len(s.logs)-1]
			b.WriteString("       " + activityStyle.Render("→ "+latest) + "\n")
		}

		// Expanded log view
		if isExpanded && len(s.logs) > 0 {
			logs := s.logs
			const maxLines = 20
			skipped := 0
			if len(logs) > maxLines {
				skipped = len(logs) - maxLines
				logs = logs[skipped:]
			}
			if skipped > 0 {
				b.WriteString("    " + hintStyle.Render(fmt.Sprintf("  ... (%d earlier lines)", skipped)) + "\n")
			}
			for _, l := range logs {
				b.WriteString("    " + logLineStyle.Render("│  "+l) + "\n")
			}
		}
	}

	// Global warnings
	if len(m.warns) > 0 {
		b.WriteString("\n")
		for _, w := range m.warns {
			b.WriteString("  " + warnStyle.Render("! "+w) + "\n")
		}
	}

	// Error message
	if m.done && m.finalErr != nil {
		b.WriteString("\n")
		for _, line := range strings.Split(m.finalErr.Error(), "\n") {
			if strings.TrimSpace(line) != "" {
				b.WriteString("  " + stepErrStyle.Render(line) + "\n")
			}
		}
	}

	// Footer
	b.WriteString("\n")
	b.WriteString(elapsedStyle.Render(fmt.Sprintf("Total: %s", m.elapsed.Round(time.Second))))
	hints := "  " + hintStyle.Render("↑↓ navigate  Enter expand/collapse  f follow current")
	b.WriteString(hints + "\n")

	return b.String()
}

// runInstallModel starts the Bubble Tea program and runs Up() in a goroutine.
func runInstallModel(ctx context.Context, cfg Config) error {
	prog := tea.NewProgram(newInstallModel())

	go func() {
		err := Up(ctx, cfg, func(ev ProgressEvent) {
			prog.Send(progressMsg(ev))
		})
		if err != nil {
			prog.Send(progressMsg(ProgressEvent{Kind: EventError, Err: err}))
		}
	}()

	_, err := prog.Run()
	return err
}
