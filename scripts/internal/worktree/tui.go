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

// Color palette — muted, modern terminal aesthetic.
var (
	colorPurple  = lipgloss.Color("99")
	colorGreen   = lipgloss.Color("42")
	colorYellow  = lipgloss.Color("214")
	colorRed     = lipgloss.Color("196")
	colorDim     = lipgloss.Color("241")
	colorFaint   = lipgloss.Color("238")
	colorCyan    = lipgloss.Color("80")
	colorWhite   = lipgloss.Color("255")
	colorSubtle  = lipgloss.Color("245")
	colorHotPink = lipgloss.Color("205")

	styleTitle = lipgloss.NewStyle().
			Bold(true).
			Foreground(colorWhite).
			Background(colorPurple).
			Padding(0, 1).
			MarginBottom(1)

	styleStatus = map[Status]lipgloss.Style{
		StatusActive:   lipgloss.NewStyle().Foreground(colorGreen),
		StatusMerged:   lipgloss.NewStyle().Foreground(colorYellow),
		StatusDetached: lipgloss.NewStyle().Foreground(colorSubtle),
		StatusPrunable: lipgloss.NewStyle().Foreground(colorRed),
	}

	// Indicator icons per status
	statusIcon = map[Status]string{
		StatusActive:   "●",
		StatusMerged:   "◆",
		StatusDetached: "○",
		StatusPrunable: "✖",
	}

	styleCheck   = lipgloss.NewStyle().Foreground(colorHotPink).Bold(true)
	styleMain    = lipgloss.NewStyle().Foreground(colorCyan).Bold(true)
	stylePath    = lipgloss.NewStyle().Foreground(colorWhite)
	styleBranch  = lipgloss.NewStyle().Foreground(colorCyan)
	styleDimPath = lipgloss.NewStyle().Foreground(colorDim)

	styleMessage = lipgloss.NewStyle().Foreground(colorGreen).Bold(true)
	styleError   = lipgloss.NewStyle().Foreground(colorRed).Bold(true)
	styleBusy    = lipgloss.NewStyle().Foreground(colorYellow).Bold(true)

	styleHelpKey  = lipgloss.NewStyle().Foreground(colorCyan).Bold(true)
	styleHelpDesc = lipgloss.NewStyle().Foreground(colorDim)
	styleHelpSep  = lipgloss.NewStyle().Foreground(colorFaint)

	styleCount = lipgloss.NewStyle().Foreground(colorSubtle)
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
	// Wider columns to accommodate ANSI color codes in cell values
	columns := []table.Column{
		{Title: " ", Width: 4},
		{Title: "Path", Width: 50},
		{Title: "Branch", Width: 40},
		{Title: "Status", Width: 18},
	}

	rows := make([]table.Row, len(entries))
	for i, e := range entries {
		rows[i] = entryToRow(e, false)
	}

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
		Bold(true).
		Foreground(colorSubtle)
	s.Selected = s.Selected.
		Foreground(colorWhite).
		Background(lipgloss.Color("236")).
		Bold(true)
	t.SetStyles(s)

	t.SetRows(rows)
	t.SetHeight(min(len(entries)+1, 25))

	return tuiModel{
		table:    t,
		entries:  entries,
		selected: make(map[int]bool),
	}
}

func entryToRow(e Entry, selected bool) table.Row {
	// Selection indicator
	check := " "
	if selected {
		check = styleCheck.Render("✓")
	}
	if e.IsMain {
		check = styleMain.Render("★")
	}

	// Path with dimmed prefix
	path := shortenPath(e.Path)
	if strings.HasPrefix(path, ".worktrees/") {
		path = styleDimPath.Render(".worktrees/") + stylePath.Render(strings.TrimPrefix(path, ".worktrees/"))
	} else {
		path = stylePath.Render(path)
	}

	// Branch with color
	branch := e.Branch
	if branch == "" {
		branch = styleDimPath.Render("(detached)")
	} else {
		branch = styleBranch.Render(branch)
	}

	// Status with icon and color
	icon := statusIcon[e.Status]
	stStyle := styleStatus[e.Status]
	status := stStyle.Render(icon + " " + e.Status.String())

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

// helpItem renders a single "key desc" help entry with styled key.
func helpItem(key, desc string) string {
	return styleHelpKey.Render(key) + " " + styleHelpDesc.Render(desc)
}

func (m tuiModel) View() tea.View {
	if m.quitting {
		return tea.NewView("")
	}

	var b strings.Builder

	// Title bar with worktree count
	selected := len(m.selected)
	title := "Worktree Manager"
	counter := styleCount.Render(fmt.Sprintf("  %d worktrees", len(m.entries)))
	if selected > 0 {
		counter = styleCheck.Render(fmt.Sprintf("  %d selected", selected))
	}
	b.WriteString(styleTitle.Render(title) + counter)
	b.WriteString("\n\n")

	// Table
	b.WriteString(m.table.View())
	b.WriteString("\n\n")

	// Status message
	if m.busy {
		b.WriteString(styleBusy.Render("  " + m.message))
		b.WriteString("\n\n")
	} else if m.err != nil {
		b.WriteString(styleError.Render("  " + m.err.Error()))
		b.WriteString("\n\n")
	} else if m.message != "" {
		b.WriteString(styleMessage.Render("  " + m.message))
		b.WriteString("\n\n")
	}

	// Help bar — grouped by function
	sep := styleHelpSep.Render(" · ")
	helpLine := strings.Join([]string{
		helpItem("space", "select") + sep + helpItem("a", "all merged"),
		helpItem("c", "clean") + sep + helpItem("C", "clean all") + sep + helpItem("d", "force del"),
		helpItem("p", "prune") + sep + helpItem("r", "refresh") + sep + helpItem("q", "quit"),
	}, styleHelpSep.Render("  │  "))
	b.WriteString("  " + helpLine)
	b.WriteString("\n")

	v := tea.NewView(b.String())
	v.AltScreen = true
	return v
}
