// tui.go implements an interactive terminal UI for worktree management.
package worktree

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

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

	styleCount     = lipgloss.NewStyle().Foreground(colorSubtle)
	styleLock      = lipgloss.NewStyle().Foreground(colorDim)
	styleToastBox  = lipgloss.NewStyle().Foreground(colorWhite).Background(lipgloss.Color("52")).Padding(0, 1)
	styleToastText = lipgloss.NewStyle().Foreground(lipgloss.Color("217"))
)

const toastDuration = 4 * time.Second

// Messages returned by async commands.
type deleteResultMsg struct {
	removed    int
	errors     []string // per-worktree errors
	freedBytes int64    // total bytes freed by successful removals
}

// sizeResultMsg delivers asynchronously computed disk sizes.
// The generation field prevents stale results from overwriting a newer entry list.
type sizeResultMsg struct {
	generation int            // must match tuiModel.generation to apply
	sizes      map[int]int64 // index → bytes
}

type pruneResultMsg struct{ err error }
type reloadResultMsg struct {
	entries []Entry
	err     error
}

// dismissToastMsg is sent by tea.Tick to auto-dismiss a toast.
type dismissToastMsg struct{ id int }

// dismissMessageMsg clears the status bar message after a delay.
type dismissMessageMsg struct{ seq int }

// toast represents a floating notification that auto-dismisses.
type toast struct {
	id   int
	text string
}

type tuiModel struct {
	table      table.Model
	entries    []Entry
	selected   map[int]bool
	message    string // status message after an action
	messageSeq int    // incremented on each new message, used for auto-dismiss
	busy       bool   // true while an async operation is running
	quitting   bool
	toasts     []toast // active toast notifications (errors)
	toastSeq   int     // auto-incrementing toast ID
	sizesLoaded bool   // true once async size computation has completed
	generation  int    // incremented on each reload, guards against stale sizeResultMsg
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

// humanSize formats a byte count into a human-readable string.
func humanSize(bytes int64) string {
	switch {
	case bytes == 0:
		return "-"
	case bytes < 1024:
		return fmt.Sprintf("%d B", bytes)
	case bytes < 1024*1024:
		return fmt.Sprintf("%.0f KB", float64(bytes)/1024)
	case bytes < 1024*1024*1024:
		return fmt.Sprintf("%.1f MB", float64(bytes)/(1024*1024))
	default:
		return fmt.Sprintf("%.1f GB", float64(bytes)/(1024*1024*1024))
	}
}

// relativeTime formats a timestamp as a human-readable relative duration.
func relativeTime(t time.Time) string {
	if t.IsZero() {
		return "-"
	}
	d := time.Since(t)
	switch {
	case d < time.Minute:
		return "just now"
	case d < time.Hour:
		return fmt.Sprintf("%dm ago", int(d.Minutes()))
	case d < 24*time.Hour:
		return fmt.Sprintf("%dh ago", int(d.Hours()))
	case d < 30*24*time.Hour:
		return fmt.Sprintf("%dd ago", int(d.Hours()/24))
	default:
		months := int(d.Hours() / 24 / 30)
		if months < 1 {
			months = 1
		}
		return fmt.Sprintf("%d mo ago", months)
	}
}

func newTUIModel(entries []Entry) tuiModel {
	// Wider columns to accommodate ANSI color codes in cell values
	columns := []table.Column{
		{Title: " ", Width: 4},
		{Title: "Path", Width: 36},
		{Title: "Branch", Width: 28},
		{Title: "Status", Width: 18},
		{Title: "Last Active", Width: 12},
		{Title: "Size", Width: 8},
	}

	rows := make([]table.Row, len(entries))
	for i, e := range entries {
		rows[i] = entryToRow(e, false, false)
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

func entryToRow(e Entry, selected bool, sizesLoaded bool) table.Row {
	// Selection indicator column
	check := " "
	if selected {
		check = styleCheck.Render("✓")
	}
	if e.IsMain {
		check = styleMain.Render("★")
	} else if e.Locked {
		check = styleLock.Render("🔒")
	} else if e.IsCurrent {
		check = styleMain.Render("▸")
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

	// Status with icon and color, add lock/current tag
	icon := statusIcon[e.Status]
	stStyle := styleStatus[e.Status]
	statusText := icon + " " + e.Status.String()
	if e.Locked {
		statusText += styleLock.Render(" 🔒")
	} else if e.IsCurrent {
		statusText += styleLock.Render(" cwd")
	}
	status := stStyle.Render(statusText)

	// Last active column
	lastActive := styleDimPath.Render(relativeTime(e.LastActive))

	// Size column — show placeholder until async computation finishes
	var size string
	if !sizesLoaded {
		size = styleDimPath.Render("...")
	} else if e.DiskSize == 0 {
		size = styleDimPath.Render("-")
	} else {
		size = styleDimPath.Render(humanSize(e.DiskSize))
	}

	return table.Row{check, path, branch, status, lastActive, size}
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

// setMessage sets the status bar message and returns a Cmd to auto-dismiss it.
func (m *tuiModel) setMessage(text string) tea.Cmd {
	m.messageSeq++
	seq := m.messageSeq
	m.message = text
	return tea.Tick(toastDuration, func(time.Time) tea.Msg {
		return dismissMessageMsg{seq: seq}
	})
}

// pushToast adds an error toast and returns a Cmd to auto-dismiss it.
func (m *tuiModel) pushToast(text string) tea.Cmd {
	m.toastSeq++
	id := m.toastSeq
	m.toasts = append(m.toasts, toast{id: id, text: text})
	return tea.Tick(toastDuration, func(time.Time) tea.Msg {
		return dismissToastMsg{id: id}
	})
}

func (m tuiModel) Init() tea.Cmd {
	return m.computeSizesCmd()
}

// computeSizesCmd returns a tea.Cmd that computes disk sizes for all entries in the background.
// Captures the current generation to discard stale results after a reload.
func (m *tuiModel) computeSizesCmd() tea.Cmd {
	entries := m.entries
	gen := m.generation
	return func() tea.Msg {
		sizes := make(map[int]int64, len(entries))
		for i, e := range entries {
			if e.Prunable {
				continue
			}
			if _, err := os.Stat(e.Path); err != nil {
				continue
			}
			sizes[i] = dirSize(e.Path)
		}
		return sizeResultMsg{generation: gen, sizes: sizes}
	}
}

func (m tuiModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	// Auto-dismiss handlers
	case dismissToastMsg:
		for i, t := range m.toasts {
			if t.id == msg.id {
				m.toasts = append(m.toasts[:i], m.toasts[i+1:]...)
				break
			}
		}
		return m, nil

	case dismissMessageMsg:
		if msg.seq == m.messageSeq {
			m.message = ""
		}
		return m, nil

	case sizeResultMsg:
		// Discard stale results from a previous generation
		if msg.generation != m.generation {
			return m, nil
		}
		for i, sz := range msg.sizes {
			if i < len(m.entries) {
				m.entries[i].DiskSize = sz
			}
		}
		m.sizesLoaded = true
		m.refreshRows()
		return m, nil

	// Async result handlers
	case deleteResultMsg:
		m.busy = false
		var cmds []tea.Cmd
		cmds = append(cmds, m.setMessage(fmt.Sprintf("Removed %d worktree(s), freed %s", msg.removed, humanSize(msg.freedBytes))))
		for _, errText := range msg.errors {
			cmds = append(cmds, m.pushToast(errText))
		}
		cmds = append(cmds, m.reloadCmd())
		return m, tea.Batch(cmds...)

	case pruneResultMsg:
		m.busy = false
		if msg.err != nil {
			return m, m.pushToast(msg.err.Error())
		}
		var cmds []tea.Cmd
		cmds = append(cmds, m.setMessage("Pruned stale worktree references"))
		cmds = append(cmds, m.reloadCmd())
		return m, tea.Batch(cmds...)

	case reloadResultMsg:
		m.busy = false
		if msg.err != nil {
			return m, m.pushToast(msg.err.Error())
		}
		m.entries = msg.entries
		m.selected = make(map[int]bool)
		m.sizesLoaded = false
		m.generation++
		m.refreshRows()
		m.table.SetHeight(min(len(msg.entries)+1, 25))
		// Re-trigger async size computation for the new entries
		sizeCmd := m.computeSizesCmd()
		// If no message was set by a prior handler (e.g. deleteResultMsg),
		// show a brief "Refreshed" note
		if m.message == "Refreshing..." {
			return m, tea.Batch(m.setMessage("Refreshed"), sizeCmd)
		}
		return m, sizeCmd

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
			// Toggle selection (skip protected worktrees)
			idx := m.table.Cursor()
			if idx < len(m.entries) && !m.entries[idx].Protected() {
				wasSelected := m.selected[idx]
				m.selected[idx] = !wasSelected
				if wasSelected {
					delete(m.selected, idx)
				} else {
					// Auto-advance cursor when selecting
					m.table.MoveDown(1)
				}
				m.refreshRows()
			}
			return m, nil

		case "a":
			// Select all merged (skip protected)
			for i, e := range m.entries {
				if e.Status == StatusMerged && !e.Protected() {
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
			// Clean ALL merged worktrees (skip protected)
			for i, e := range m.entries {
				if e.Status == StatusMerged && !e.Protected() {
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
	type target struct {
		path     string
		branch   string
		diskSize int64 // cached size from entry, or computed fresh if not loaded
	}
	var targets []target
	for idx, sel := range m.selected {
		if !sel || idx >= len(m.entries) {
			continue
		}
		e := m.entries[idx]
		if e.Protected() {
			continue
		}
		targets = append(targets, target{path: e.Path, branch: e.Branch, diskSize: e.DiskSize})
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
		var freedBytes int64
		var errors []string
		for _, t := range targets {
			// Use cached size if available, otherwise compute fresh
			sz := t.diskSize
			if sz == 0 {
				sz = dirSize(t.path)
			}
			if err := Remove(t.path, force); err != nil {
				errors = append(errors, fmt.Sprintf("%s: %s", shortenPath(t.path), err))
				continue
			}
			freedBytes += sz
			if t.branch != "" {
				_ = DeleteBranch(t.branch, force)
			}
			removed++
		}
		_ = Prune()
		return deleteResultMsg{removed: removed, errors: errors, freedBytes: freedBytes}
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
		rows[i] = entryToRow(e, m.selected[i], m.sizesLoaded)
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
	} else if m.message != "" {
		b.WriteString(styleMessage.Render("  " + m.message))
		b.WriteString("\n\n")
	}

	// Floating toast notifications (errors)
	for _, t := range m.toasts {
		b.WriteString(styleToastBox.Render(styleToastText.Render("  " + t.text)))
		b.WriteString("\n")
	}
	if len(m.toasts) > 0 {
		b.WriteString("\n")
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
