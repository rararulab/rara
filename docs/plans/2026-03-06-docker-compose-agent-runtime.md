# Docker Compose Agent Runtime Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add Docker + Docker Compose support so rara agent can run containerized and be validated in chat mode.

**Architecture:** Reuse existing multi-stage backend Dockerfile to build a hardened runtime image, add a dedicated compose file with isolated runtime settings, and provide scripts + just rules for image build/update/cleanup lifecycle. Validate by running `chat` in the container with a self-contained SQLite runtime.

**Tech Stack:** Docker, Docker Compose, Just, Bash, Rust CLI (`rara`)

---

### Task 1: Add Compose Runtime for Containerized Agent

**Files:**
- Create: `docker-compose.agent.yml`

**Steps:**
1. Define `rara-agent` service built from `docker/Dockerfile`.
2. Set runtime env for writable tmpfs-backed data/config paths.
3. Harden container runtime (`read_only`, `tmpfs`, dropped caps, `no-new-privileges`, resource limits).
4. Keep host filesystem unmounted.

### Task 2: Add Build/Update/Clean Scripts

**Files:**
- Create: `scripts/docker-agent-build.sh`
- Create: `scripts/docker-agent-up.sh`
- Create: `scripts/docker-agent-clean.sh`

**Steps:**
1. Add strict shell options and repo-root resolution.
2. Build script: compose build for `rara-agent`.
3. Update script: compose up with force recreate and detached mode.
4. Clean script: compose down with volume and image cleanup options.

### Task 3: Expose Justfile Rules and Document Usage

**Files:**
- Modify: `justfile`
- Modify: `README.md`

**Steps:**
1. Add just recipes wrapping the three scripts.
2. Add chat-mode run recipe using compose interactive run.
3. Add README section with quickstart and isolation notes.

### Task 4: Verify End-to-End in Container

**Files:**
- Verify only

**Steps:**
1. Run `just docker-agent-build`.
2. Run `just docker-agent-up`.
3. Run compose chat-mode command and confirm containerized CLI starts interactive mode and exits cleanly on EOF.
4. Run cleanup command and record status.
