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
	"fmt"
	"os"
	"strings"
)

const (
	hostsBeginMarker = "# BEGIN rara-setup"
	hostsEndMarker   = "# END rara-setup"
	hostsFile        = "/etc/hosts"
)

// AddHostsEntries adds (or replaces) the rara-setup block in /etc/hosts.
// It writes entries for the given domain and common sub-domains pointing to the given IP.
func AddHostsEntries(ip, domain string) error {
	current, err := os.ReadFile(hostsFile)
	if err != nil {
		return fmt.Errorf("read %s: %w", hostsFile, err)
	}

	block := buildHostsBlock(ip, domain)
	updated := replaceHostsBlock(string(current), block)

	// Write back with same permissions
	info, err := os.Stat(hostsFile)
	if err != nil {
		return fmt.Errorf("stat %s: %w", hostsFile, err)
	}

	if err := os.WriteFile(hostsFile, []byte(updated), info.Mode()); err != nil {
		return fmt.Errorf("write %s: %w", hostsFile, err)
	}

	return nil
}

// RemoveHostsEntries removes the rara-setup block from /etc/hosts.
func RemoveHostsEntries() error {
	current, err := os.ReadFile(hostsFile)
	if err != nil {
		return fmt.Errorf("read %s: %w", hostsFile, err)
	}

	updated := removeHostsBlock(string(current))

	info, err := os.Stat(hostsFile)
	if err != nil {
		return fmt.Errorf("stat %s: %w", hostsFile, err)
	}

	if err := os.WriteFile(hostsFile, []byte(updated), info.Mode()); err != nil {
		return fmt.Errorf("write %s: %w", hostsFile, err)
	}

	return nil
}

// PrintHostsBlock prints the proposed /etc/hosts block to stdout (dry-run).
func PrintHostsBlock(ip, domain string) {
	fmt.Println(buildHostsBlock(ip, domain))
}

// buildHostsBlock constructs the hosts block for the given IP and domain.
func buildHostsBlock(ip, domain string) string {
	subdomains := []string{
		domain,
		"traefik." + domain,
		"grafana." + domain,
		"consul." + domain,
		"minio." + domain,
		"memos." + domain,
		"ollama." + domain,
	}

	var sb strings.Builder
	sb.WriteString(hostsBeginMarker + "\n")
	for _, host := range subdomains {
		sb.WriteString(fmt.Sprintf("%s\t%s\n", ip, host))
	}
	sb.WriteString(hostsEndMarker + "\n")
	return sb.String()
}

// replaceHostsBlock replaces the existing rara-setup block or appends a new one.
func replaceHostsBlock(content, block string) string {
	beginIdx := strings.Index(content, hostsBeginMarker)
	endIdx := strings.Index(content, hostsEndMarker)

	if beginIdx != -1 && endIdx != -1 && endIdx > beginIdx {
		// Replace the existing block (including the end marker line)
		endLine := endIdx + len(hostsEndMarker)
		// Move past the trailing newline if present
		if endLine < len(content) && content[endLine] == '\n' {
			endLine++
		}
		return content[:beginIdx] + block + content[endLine:]
	}

	// Append the block
	if !strings.HasSuffix(content, "\n") {
		content += "\n"
	}
	return content + block
}

// removeHostsBlock removes the rara-setup block from content.
func removeHostsBlock(content string) string {
	beginIdx := strings.Index(content, hostsBeginMarker)
	endIdx := strings.Index(content, hostsEndMarker)

	if beginIdx == -1 || endIdx == -1 || endIdx <= beginIdx {
		return content
	}

	endLine := endIdx + len(hostsEndMarker)
	if endLine < len(content) && content[endLine] == '\n' {
		endLine++
	}
	return content[:beginIdx] + content[endLine:]
}
