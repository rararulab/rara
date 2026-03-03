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
	"bytes"
	"compress/gzip"
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"sort"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/kubernetes"
)

// helmRelease is the minimal structure of a Helm release payload.
type helmRelease struct {
	Config map[string]interface{} `json:"config"`
}

// ReadHelmValues reads user-supplied values from the latest deployed Helm release secret.
//
// Helm 3 stores releases as K8s Secrets named sh.helm.release.v1.<name>.v<N>.
// The .Data["release"] field is decoded as: base64 (Helm layer) -> gzip -> JSON.
// (The K8s Secret layer base64 is already decoded by client-go.)
func ReadHelmValues(ctx context.Context, cs *kubernetes.Clientset, namespace, release string) (HelmValues, error) {
	secrets, err := cs.CoreV1().Secrets(namespace).List(ctx, metav1.ListOptions{
		LabelSelector: fmt.Sprintf("owner=helm,name=%s,status=deployed", release),
	})
	if err != nil {
		return HelmValues{}, fmt.Errorf("list helm secrets: %w", err)
	}
	if len(secrets.Items) == 0 {
		return HelmValues{}, fmt.Errorf("no deployed helm release found for %q", release)
	}

	// Sort by name descending to get the latest version (e.g. ...v3 > ...v2).
	sort.Slice(secrets.Items, func(i, j int) bool {
		return secrets.Items[i].Name > secrets.Items[j].Name
	})

	releaseData, ok := secrets.Items[0].Data["release"]
	if !ok {
		return HelmValues{}, fmt.Errorf("release key not found in secret %s", secrets.Items[0].Name)
	}

	rel, err := decodeRelease(releaseData)
	if err != nil {
		return HelmValues{}, fmt.Errorf("decode release: %w", err)
	}

	return extractHelmValues(rel.Config), nil
}

// decodeRelease decodes the Helm release payload from a K8s Secret's .Data["release"].
// Chain: base64 (Helm layer) -> gzip -> JSON.
func decodeRelease(data []byte) (*helmRelease, error) {
	// Helm's own base64 encoding.
	decoded, err := base64.StdEncoding.DecodeString(string(data))
	if err != nil {
		return nil, fmt.Errorf("helm base64 decode: %w", err)
	}

	// Gzip decompression.
	gz, err := gzip.NewReader(bytes.NewReader(decoded))
	if err != nil {
		return nil, fmt.Errorf("gzip reader: %w", err)
	}
	defer gz.Close()

	var buf bytes.Buffer
	if _, err := buf.ReadFrom(gz); err != nil {
		return nil, fmt.Errorf("gzip decompress: %w", err)
	}

	var rel helmRelease
	if err := json.Unmarshal(buf.Bytes(), &rel); err != nil {
		return nil, fmt.Errorf("json unmarshal: %w", err)
	}

	return &rel, nil
}

// extractHelmValues maps the raw Helm config map to our HelmValues struct.
func extractHelmValues(config map[string]interface{}) HelmValues {
	defaults := DefaultHelmValues()
	return HelmValues{
		Mem0Enabled:      nestedBool(config, defaults.Mem0Enabled, "mem0", "enabled"),
		MemosEnabled:     nestedBool(config, defaults.MemosEnabled, "memos", "enabled"),
		HindsightEnabled: nestedBool(config, defaults.HindsightEnabled, "hindsight", "enabled"),
		OllamaEnabled:    nestedBool(config, defaults.OllamaEnabled, "ollama", "enabled"),
		Domain:           nestedString(config, defaults.Domain, "global", "domain"),
	}
}

// nestedBool traverses nested maps by keys and returns a bool, or defaultVal if not found.
func nestedBool(m map[string]interface{}, defaultVal bool, keys ...string) bool {
	v := nestedGet(m, keys...)
	if v == nil {
		return defaultVal
	}
	if b, ok := v.(bool); ok {
		return b
	}
	return defaultVal
}

// nestedString traverses nested maps by keys and returns a string, or defaultVal if not found.
func nestedString(m map[string]interface{}, defaultVal string, keys ...string) string {
	v := nestedGet(m, keys...)
	if v == nil {
		return defaultVal
	}
	if s, ok := v.(string); ok {
		return s
	}
	return defaultVal
}

// nestedGet traverses nested maps by keys and returns the final value, or nil.
func nestedGet(m map[string]interface{}, keys ...string) interface{} {
	current := interface{}(m)
	for _, key := range keys {
		cm, ok := current.(map[string]interface{})
		if !ok {
			return nil
		}
		current, ok = cm[key]
		if !ok {
			return nil
		}
	}
	return current
}
