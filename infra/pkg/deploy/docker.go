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

package deploy

import (
	"fmt"
	"os"
	"os/exec"

	"github.com/google/go-containerregistry/pkg/authn"
	"github.com/google/go-containerregistry/pkg/crane"
)

// ImageExists checks if a Docker image exists in the remote registry via OCI HEAD request.
func ImageExists(ref string) bool {
	_, err := crane.Head(ref, crane.WithAuthFromKeychain(authn.DefaultKeychain))
	return err == nil
}

// Build runs docker build with the given options.
func Build(dockerfile string, tags []string, buildArgs map[string]string) error {
	args := []string{"build", "--file", dockerfile}
	for _, tag := range tags {
		args = append(args, "--tag", tag)
	}
	for k, v := range buildArgs {
		args = append(args, "--build-arg", fmt.Sprintf("%s=%s", k, v))
	}
	args = append(args, ".")

	cmd := exec.Command("docker", args...)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

// Push pushes a Docker image to the registry.
func Push(ref string) error {
	cmd := exec.Command("docker", "push", ref)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}
