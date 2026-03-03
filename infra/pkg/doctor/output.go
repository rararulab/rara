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

package doctor

import (
	"fmt"
	"io"
	"os"

	"golang.org/x/term"
)

const (
	colorReset  = "\033[0m"
	colorGreen  = "\033[32m"
	colorRed    = "\033[31m"
	colorYellow = "\033[33m"
	colorGray   = "\033[90m"
)

// Printer formats and writes doctor reports to a writer.
type Printer struct {
	w     io.Writer
	color bool
}

// NewPrinter creates a Printer that auto-detects color support.
func NewPrinter(w io.Writer) *Printer {
	color := false
	if f, ok := w.(*os.File); ok {
		color = term.IsTerminal(int(f.Fd()))
	}
	return &Printer{w: w, color: color}
}

// PrintReport writes the full doctor report to the writer.
func (p *Printer) PrintReport(r *Report) {
	fmt.Fprintln(p.w)
	fmt.Fprintf(p.w, "🩺 rara-infra doctor — namespace: %s\n", r.Namespace)
	p.line()

	for _, section := range r.Sections {
		fmt.Fprintln(p.w)
		fmt.Fprintf(p.w, "%s %s\n", section.Icon, section.Title)
		for _, check := range section.Checks {
			icon := p.icon(check.Status)
			fmt.Fprintf(p.w, " %s  %-28s %s\n", icon, check.Name, check.Detail)
		}
	}

	// Summary
	pass, fail, skip := r.Summary()
	fmt.Fprintln(p.w)
	p.line()
	fmt.Fprintf(p.w, "  %s %d passed", p.colorize(colorGreen, "✔"), pass)
	if fail > 0 {
		fmt.Fprintf(p.w, "  %s %d failed", p.colorize(colorRed, "✘"), fail)
	}
	if skip > 0 {
		fmt.Fprintf(p.w, "  %s %d skipped", p.colorize(colorGray, "–"), skip)
	}
	fmt.Fprintln(p.w)
}

func (p *Printer) line() {
	fmt.Fprintln(p.w, "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
}

func (p *Printer) icon(s Status) string {
	switch s {
	case StatusPass:
		return p.colorize(colorGreen, "✔")
	case StatusFail:
		return p.colorize(colorRed, "✘")
	case StatusSkip:
		return p.colorize(colorGray, "–")
	case StatusWarn:
		return p.colorize(colorYellow, "⚠")
	default:
		return "?"
	}
}

func (p *Printer) colorize(color, text string) string {
	if !p.color {
		return text
	}
	return color + text + colorReset
}
