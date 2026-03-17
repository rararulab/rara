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
	styleBusy    = lipgloss.NewStyle().Foreground(lipgloss.Color("214")).Bold(true)
)

// Messages returned by async commands.
type deleteResultMsg struct {
	removed int
	err     error
}

type pruneResultMsg struct{ err error }
type reloadResultMsg struct {
	entries []Entry
	err     error
}

type tuiModel struct {
	table    table.Model
	entries  []Entry
	selected map[int]bool
	message  string // status message after an action
	err      error
	busy     bool // true while an async operation is running
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

	// Total width = sum of column widths + padding (2 per col)
	totalWidth := 0
	for _, c := range columns {
		totalWidth += c.Width + 2
	}

	t := table.New(
		table.WithColumns(columns),
		table.WithWidth(totalWidth),
		table.WithFocused(true),
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

	// Set rows and height AFTER styles are applied so viewport calculates correctly
	t.SetRows(rows)
	t.SetHeight(min(len(entries)+1, 25))

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
	// Async result handlers
	case deleteResultMsg:
		m.busy = false
		if msg.err != nil {
			m.err = msg.err
		} else {
			m.message = fmt.Sprintf("Removed %d worktree(s)", msg.removed)
			m.err = nil
		}
		return m, m.reloadCmd()

	case pruneResultMsg:
		m.busy = false
		if msg.err != nil {
			m.err = msg.err
		} else {
			m.message = "Pruned stale worktree references"
		}
		return m, m.reloadCmd()

	case reloadResultMsg:
		m.busy = false
		if msg.err != nil {
			m.err = msg.err
			return m, nil
		}
		m.entries = msg.entries
		m.selected = make(map[int]bool)
		m.refreshRows()
		m.table.SetHeight(min(len(msg.entries)+1, 25))
		return m, nil

	case tea.KeyPressMsg:
		// Ignore keys while busy
		if m.busy {
			return m, nil
		}
		switch msg.String() {
		case "q", "ctrl+c":
			m.quitting = true
			return m, tea.Quit

		case "space":
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
			return m, m.deleteSelectedCmd(true)

		case "c":
			// Clean selected merged worktrees
			return m, m.deleteSelectedCmd(false)

		case "C":
			// Clean ALL merged worktrees
			for i, e := range m.entries {
				if e.Status == StatusMerged && !e.IsMain {
					m.selected[i] = true
				}
			}
			return m, m.deleteSelectedCmd(false)

		case "p":
			// Prune stale references
			m.busy = true
			m.message = "Pruning..."
			return m, func() tea.Msg {
				err := Prune()
				return pruneResultMsg{err: err}
			}

		case "r":
			// Refresh list
			m.busy = true
			m.message = "Refreshing..."
			return m, m.reloadCmd()
		}
	}

	var cmd tea.Cmd
	m.table, cmd = m.table.Update(msg)
	return m, cmd
}

// deleteSelectedCmd returns a tea.Cmd that removes selected worktrees in the background.
func (m *tuiModel) deleteSelectedCmd(force bool) tea.Cmd {
	// Snapshot the work to do before going async
	type target struct {
		path   string
		branch string
	}
	var targets []target
	for idx, sel := range m.selected {
		if !sel || idx >= len(m.entries) {
			continue
		}
		e := m.entries[idx]
		if e.IsMain {
			continue
		}
		targets = append(targets, target{path: e.Path, branch: e.Branch})
	}
	if len(targets) == 0 {
		return nil
	}

	m.busy = true
	m.message = fmt.Sprintf("Removing %d worktree(s)...", len(targets))
	m.selected = make(map[int]bool)
	m.refreshRows()

	return func() tea.Msg {
		removed := 0
		var lastErr error
		for _, t := range targets {
			if err := Remove(t.path, force); err != nil {
				lastErr = fmt.Errorf("remove %s: %w", shortenPath(t.path), err)
				continue
			}
			if t.branch != "" {
				_ = DeleteBranch(t.branch, force)
			}
			removed++
		}
		_ = Prune()
		return deleteResultMsg{removed: removed, err: lastErr}
	}
}

// reloadCmd returns a tea.Cmd that refreshes the worktree list in the background.
func (m *tuiModel) reloadCmd() tea.Cmd {
	m.busy = true
	return func() tea.Msg {
		entries, err := List()
		return reloadResultMsg{entries: entries, err: err}
	}
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
	if m.busy {
		b.WriteString(styleBusy.Render("⏳ " + m.message))
		b.WriteString("\n")
	} else if m.err != nil {
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
