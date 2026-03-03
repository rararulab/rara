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

// Config holds all configuration for the kind-based local setup.
type Config struct {
	ClusterName      string
	Namespace        string
	Domain           string
	InfraRelease     string
	AppRelease       string
	PostgresPassword string
	PostgresDatabase string
	MinioUser        string
	MinioPassword    string
	EnableOllama     bool
	EnableMemos      bool
	EnableHindsight  bool
	EnableMem0       bool

	HindsightLLMProvider string
	HindsightLLMModel    string
	HindsightLLMBaseURL  string

	Mem0OllamaBaseURL string
	Mem0OllamaModel   string

}

// DefaultConfig returns a Config with sensible defaults for local dev.
func DefaultConfig() Config {
	return Config{
		ClusterName:      "rara",
		Namespace:        "rara",
		Domain:           "rara.local",
		InfraRelease:     "rara-infra",
		AppRelease:       "rara-app",
		PostgresPassword: "postgres",
		PostgresDatabase: "rara",
		MinioUser:        "minioadmin",
		MinioPassword:    "minioadmin",
		EnableOllama:     true,
		EnableMemos:      true,
		EnableHindsight:  true,
		EnableMem0:       true,

		HindsightLLMProvider: "ollama",
		HindsightLLMModel:    "qwen3:latest",
		HindsightLLMBaseURL:  "http://rara-infra-ollama:11434/v1",

		Mem0OllamaBaseURL: "http://rara-infra-ollama:11434",
		Mem0OllamaModel:   "qwen3:latest",
	}
}

// Prefix returns the resource name prefix for the infra stack.
func (c *Config) Prefix() string { return c.InfraRelease }
