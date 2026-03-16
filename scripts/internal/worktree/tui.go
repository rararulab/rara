// tui.go implements an interactive terminal UI for worktree management.
package worktree

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	tea "charm.land/bubbletea/v2"
	"charm.land/bubbles/v2/table"
	"charm.land/lipgloss/v2"
)

var (
	styleTitle = lipgloss.NewStyle().
			Bold(true).
			Foreground(lipgloss.Color("229")).
			Background(lipgloss.Color("57")).
			Padding(0, 1)

	styleStatus = map[Status]lipgloss.Style{
		StatusActive:   lipgloss.NewStyle().Foreground(lipgloss.Color("42")),
		StatusMerged:   lipgloss.NewStyle().Foreground(lipgloss.Color("214")),
		StatusDetached: lipgloss.NewStyle().Foreground(lipgloss.Color("245")),
		StatusPrunable: lipgloss.NewStyle().Foreground(lipgloss.Color("196")),
	}

	styleHelp    = lipgloss.NewStyle().Foreground(lipgloss.Color("241"))
	styleMessage = lipgloss.NewStyle().Foreground(lipgloss.Color("42")).Bold(true)
	styleError   = lipgloss.NewStyle().Foreground(lipgloss.Color("196")).Bold(true)
)

type tuiModel struct {
	table    table.Model
	entries  []Entry
	selected map[int]bool
	message  string // status message after an action
	err      error
	quitting bool
}

// RunTUI launches the interactive worktree manager.
func RunTUI() error {
	entries, err := List()
	if err != nil {
		return err
	}

	m := newTUIModel(entries)
	p := tea.NewProgram(m)
	if _, err := p.Run(); err != nil {
		return fmt.Errorf("TUI error: %w", err)
	}
	return nil
}

func newTUIModel(entries []Entry) tuiModel {
	columns := []table.Column{
		{Title: " ", Width: 3},
		{Title: "Path", Width: 45},
		{Title: "Branch", Width: 35},
		{Title: "Status", Width: 10},
	}

	rows := make([]table.Row, len(entries))
	for i, e := range entries {
		rows[i] = entryToRow(e, false)
	}

	t := table.New(
		table.WithColumns(columns),
		table.WithRows(rows),
		table.WithFocused(true),
		table.WithHeight(min(len(entries)+1, 25)),
	)

	s := table.DefaultStyles()
	s.Header = s.Header.
		BorderStyle(lipgloss.NormalBorder()).
		BorderBottom(true).
		Bold(true)
	s.Selected = s.Selected.
		Foreground(lipgloss.Color("229")).
		Background(lipgloss.Color("57")).
		Bold(false)
	t.SetStyles(s)

	return tuiModel{
		table:    t,
		entries:  entries,
		selected: make(map[int]bool),
	}
}

func entryToRow(e Entry, selected bool) table.Row {
	check := " "
	if selected {
		check = "✓"
	}
	if e.IsMain {
		check = "●"
	}

	path := shortenPath(e.Path)
	branch := e.Branch
	if branch == "" {
		branch = "(detached)"
	}

	status := e.Status.String()
	return table.Row{check, path, branch, status}
}

func shortenPath(p string) string {
	home, err := os.UserHomeDir()
	if err == nil {
		p = strings.Replace(p, home, "~", 1)
	}
	// further shorten .worktrees/ prefix
	if idx := strings.Index(p, ".worktrees/"); idx >= 0 {
		p = ".worktrees/" + filepath.Base(p)
	}
	return p
}

func (m tuiModel) Init() tea.Cmd {
	return nil
}

func (m tuiModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyPressMsg:
		switch msg.String() {
		case "q", "ctrl+c":
			m.quitting = true
			return m, tea.Quit

		case " ":
			// Toggle selection (skip main worktree)
			idx := m.table.Cursor()
			if idx < len(m.entries) && !m.entries[idx].IsMain {
				m.selected[idx] = !m.selected[idx]
				if !m.selected[idx] {
					delete(m.selected, idx)
				}
				m.refreshRows()
			}
			return m, nil

		case "a":
			// Select all merged
			for i, e := range m.entries {
				if e.Status == StatusMerged && !e.IsMain {
					m.selected[i] = true
				}
			}
			m.refreshRows()
			return m, nil

		case "d":
			// Delete selected worktrees (force)
			m.deleteSelected(true)
			return m, nil

		case "c":
			// Clean selected merged worktrees
			m.deleteSelected(false)
			return m, nil

		case "C":
			// Clean ALL merged worktrees
			for i, e := range m.entries {
				if e.Status == StatusMerged && !e.IsMain {
					m.selected[i] = true
				}
			}
			m.deleteSelected(false)
			return m, nil

		case "p":
			// Prune stale references
			if err := Prune(); err != nil {
				m.err = err
			} else {
				m.message = "Pruned stale worktree references"
				m.reload()
			}
			return m, nil

		case "r":
			// Refresh list
			m.reload()
			m.message = "Refreshed"
			return m, nil
		}
	}

	var cmd tea.Cmd
	m.table, cmd = m.table.Update(msg)
	return m, cmd
}

func (m *tuiModel) deleteSelected(force bool) {
	removed := 0
	for idx, sel := range m.selected {
		if !sel || idx >= len(m.entries) {
			continue
		}
		e := m.entries[idx]
		if e.IsMain {
			continue
		}
		if err := Remove(e.Path, force); err != nil {
			m.err = fmt.Errorf("remove %s: %w", shortenPath(e.Path), err)
			continue
		}
		if e.Branch != "" {
			_ = DeleteBranch(e.Branch, force)
		}
		removed++
	}
	_ = Prune()
	m.selected = make(map[int]bool)
	m.reload()
	m.message = fmt.Sprintf("Removed %d worktree(s)", removed)
	m.err = nil
}

func (m *tuiModel) reload() {
	entries, err := List()
	if err != nil {
		m.err = err
		return
	}
	m.entries = entries
	m.selected = make(map[int]bool)
	m.refreshRows()

	// Adjust table height
	m.table.SetHeight(min(len(entries)+1, 25))
}

func (m *tuiModel) refreshRows() {
	rows := make([]table.Row, len(m.entries))
	for i, e := range m.entries {
		rows[i] = entryToRow(e, m.selected[i])
	}
	m.table.SetRows(rows)
}

func (m tuiModel) View() tea.View {
	if m.quitting {
		return tea.NewView("")
	}

	var b strings.Builder

	b.WriteString(styleTitle.Render("🌳 Worktree Manager"))
	b.WriteString("\n\n")
	b.WriteString(m.table.View())
	b.WriteString("\n\n")

	// Status message
	if m.err != nil {
		b.WriteString(styleError.Render("Error: " + m.err.Error()))
		b.WriteString("\n")
	} else if m.message != "" {
		b.WriteString(styleMessage.Render(m.message))
		b.WriteString("\n")
	}

	// Help bar
	help := []string{
		"space: select",
		"a: select merged",
		"c: clean selected",
		"C: clean all merged",
		"d: force delete",
		"p: prune",
		"r: refresh",
		"q: quit",
	}
	b.WriteString(styleHelp.Render(strings.Join(help, " │ ")))
	b.WriteString("\n")

	v := tea.NewView(b.String())
	v.AltScreen = true
	return v
}
