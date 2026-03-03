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
	"time"
)

// Step prints a numbered step header.
func Step(n int, total int, msg string) {
	fmt.Printf("\n\033[1;34m[%d/%d]\033[0m %s\n", n, total, msg)
}

// OK prints a success line.
func OK(msg string) {
	fmt.Printf("  \033[32m+\033[0m %s\n", msg)
}

// Info prints an info line.
func Info(msg string) {
	fmt.Printf("  \033[90m->\033[0m %s\n", msg)
}

// Warn prints a warning line.
func Warn(msg string) {
	fmt.Printf("  \033[33m!\033[0m %s\n", msg)
}

// Wait prints a waiting message, calls f, then prints result.
func Wait(msg string, f func() error) error {
	fmt.Printf("  \033[90m...\033[0m %s", msg)
	start := time.Now()
	err := f()
	if err != nil {
		fmt.Printf(" \033[31mFAIL\033[0m (%v)\n", err)
		return err
	}
	fmt.Printf(" \033[32mdone\033[0m (%s)\n", time.Since(start).Round(time.Millisecond))
	return nil
}
