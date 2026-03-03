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
			huh.NewNote().Title("Langfuse (optional)"),
			huh.NewInput().Title("Langfuse Public Key").Value(&cfg.LangfusePublicKey).Placeholder("(optional)"),
			huh.NewInput().Title("Langfuse Secret Key").Value(&cfg.LangfuseSecretKey).EchoMode(huh.EchoModePassword).Placeholder("(optional)"),
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
}

type installModel struct {
	steps    []stepState
	current  int
	spinner  spinner.Model
	start    time.Time
	elapsed  time.Duration
	done     bool
	finalErr error
	logs     []string // recent info/warn lines, max 5
	width    int
}

// tea.Msg types
type progressMsg ProgressEvent
type tickMsg time.Time

var (
	stepDoneStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("42"))
	stepRunStyle  = lipgloss.NewStyle().Foreground(lipgloss.Color("33"))
	stepPendStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("240"))
	stepErrStyle  = lipgloss.NewStyle().Foreground(lipgloss.Color("160"))
	titleStyle    = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("205"))
	elapsedStyle  = lipgloss.NewStyle().Foreground(lipgloss.Color("240"))
	logWarnStyle  = lipgloss.NewStyle().Foreground(lipgloss.Color("214"))
	logInfoStyle  = lipgloss.NewStyle().Foreground(lipgloss.Color("245"))
)

func newInstallModel() installModel {
	s := spinner.New(spinner.WithSpinner(spinner.Dot))
	s.Style = stepRunStyle
	return installModel{
		spinner: s,
		start:   time.Now(),
		steps:   []stepState{},
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
		if msg.Type == tea.KeyCtrlC {
			return m, tea.Quit
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
			// Ensure steps slice is large enough
			for len(m.steps) < ev.N {
				m.steps = append(m.steps, stepState{})
			}
			m.steps[ev.N-1] = stepState{name: ev.Name, status: stepRunning}
			m.current = ev.N - 1
		case EventStepDone:
			if ev.N-1 < len(m.steps) {
				m.steps[ev.N-1].status = stepDone
				m.steps[ev.N-1].elapsed = ev.Elapsed
			}
		case EventInfo:
			m.logs = appendLog(m.logs, logInfoStyle.Render("-> "+ev.Name))
		case EventWarn:
			m.logs = appendLog(m.logs, logWarnStyle.Render("! "+ev.Name))
		case EventDone:
			m.done = true
			return m, tea.Quit
		case EventError:
			m.finalErr = ev.Err
			m.done = true
			if m.current < len(m.steps) {
				m.steps[m.current].status = stepError
			}
			return m, tea.Quit
		}
	}
	return m, nil
}

func appendLog(logs []string, line string) []string {
	logs = append(logs, line)
	if len(logs) > 5 {
		logs = logs[len(logs)-5:]
	}
	return logs
}

func (m installModel) View() string {
	var b strings.Builder

	title := titleStyle.Render("rara local setup")
	if m.done {
		if m.finalErr != nil {
			title += "  " + stepErrStyle.Render("-- error")
		} else {
			title += "  " + stepDoneStyle.Render("-- done")
		}
	} else {
		title += "  " + stepRunStyle.Render("-- installing")
	}
	b.WriteString(title + "\n\n")

	for _, s := range m.steps {
		var icon string
		var nameStyle lipgloss.Style
		switch s.status {
		case stepDone:
			icon = stepDoneStyle.Render("v")
			nameStyle = stepDoneStyle
		case stepRunning:
			icon = m.spinner.View()
			nameStyle = stepRunStyle
		case stepError:
			icon = stepErrStyle.Render("x")
			nameStyle = stepErrStyle
		default:
			icon = stepPendStyle.Render(".")
			nameStyle = stepPendStyle
		}
		line := fmt.Sprintf("  %s  %s", icon, nameStyle.Render(s.name))
		if s.status == stepDone {
			line += "  " + elapsedStyle.Render(s.elapsed.Round(time.Millisecond).String())
		}
		b.WriteString(line + "\n")
	}

	if len(m.logs) > 0 {
		b.WriteString("\n")
		for _, l := range m.logs {
			b.WriteString("     " + l + "\n")
		}
	}

	b.WriteString("\n" + elapsedStyle.Render(fmt.Sprintf("Elapsed: %s", m.elapsed.Round(time.Second))))

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
