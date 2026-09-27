##@ Security (deterministic scanners; the sec lane in CI)
.PHONY: security security.tools

# gitleaks on the history and on the tracked files (.gitleaks.toml), cargo
# audit (RustSec advisories), cargo deny (advisories, bans, sources;
# engine/deny.toml). Every tool under a timeout, over tracked files only.
# `timeout` is not on macOS; perl is everywhere.
TIMEOUT := perl -e 'alarm shift @ARGV; exec @ARGV or die "$$!"' --

security: ## Secrets, advisories, cargo deny: deterministic, under a minute
	@echo "== gitleaks (history, then tracked files)"
	@if command -v gitleaks >/dev/null; then \
		$(TIMEOUT) 120 gitleaks git --config .gitleaks.toml --redact --no-banner . && \
		$(TIMEOUT) 120 gitleaks dir --config .gitleaks.toml --redact --no-banner .; \
	else echo "gitleaks missing: make security.tools (skipped)"; fi
	@echo "== cargo audit, cargo deny"
	@if command -v cargo-audit >/dev/null; then (cd engine && $(TIMEOUT) 120 cargo audit); \
	else echo "cargo-audit missing: cargo install cargo-audit --locked (skipped)"; fi
	@if command -v cargo-deny >/dev/null; then (cd engine && $(TIMEOUT) 120 cargo deny --all-features check advisories bans sources); \
	else echo "cargo-deny missing: cargo install cargo-deny --locked (skipped)"; fi
	@echo "security: clean"

security.tools: ## Install the scanners (gitleaks, cargo-audit, cargo-deny)
	@command -v gitleaks >/dev/null || { command -v brew >/dev/null && brew install gitleaks; } \
		|| { echo "gitleaks: no Homebrew here; the pinned release ci.yml installs, or your package manager"; exit 1; }
	@command -v cargo-audit >/dev/null || cargo install cargo-audit --locked
	@command -v cargo-deny >/dev/null || cargo install cargo-deny --locked
