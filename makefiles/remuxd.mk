##@ remuxd (the engine, Rust, runs on the host)

.PHONY: remuxd.lint remuxd.tests obs.fetch remuxd.build.obs remuxd.start remuxd.identity remuxd.deps remuxd.check remuxd.test remuxd.cover remuxd.seam  remuxd.build remuxd.run

remuxd.deps: ## What the engine needs on this machine
	@command -v cargo >/dev/null || { echo "cargo missing: https://rustup.rs"; exit 1; }
	@command -v cargo-llvm-cov >/dev/null || { echo "cargo-llvm-cov missing: cargo install cargo-llvm-cov"; exit 1; }
	@command -v cargo-nextest >/dev/null || { echo "cargo-nextest missing: cargo install cargo-nextest"; exit 1; }
	@echo "ok: $$(cargo --version), $$(cargo llvm-cov --version), $$(cargo nextest --version)"

# Signing with our own certificate is not a nicety. **macOS ties a screen
# recording grant to the code signature.** Left ad hoc, cargo's linker-signed
# binary carries an identifier that is a hash of itself, so every rebuild is a
# different application: the toggle in System Settings stays on,
# `CGPreflightScreenCaptureAccess` answers yes, and the capture hands over
# nothing at all.
#
# With a certificate the requirement is the identifier plus the leaf, and both
# survive a rebuild.
ENGINE_ID := com.remux.engine

remuxd.identity: ## Create the code-signing certificate the engine is signed with (once, in the login keychain)
	@scripts/identity "$(SIGN_ID)"

# The engine's one motor here is libobs, on every host. On macOS the OBS this build links against is the one fetched
# beside the build (`make obs.fetch`), pinned by version, never the
# operator's own OBS.app.
HOST_OS := $(shell uname -s)
ifeq ($(HOST_OS),Darwin)
OBS_APP ?= $(CURDIR)/engine/target/obs/OBS.app
export OBS_APP
else
# The motor's bindings link the distribution's libobs from its folder (the
# libobs-dev package's libobs.so), not through pkg-config.
LIBOBS_PATH ?= $(patsubst %/,%,$(dir $(firstword $(wildcard /usr/lib/*/libobs.so /usr/lib64/libobs.so /usr/lib/libobs.so))))
export LIBOBS_PATH
endif
OBS_VERSION ?= 32.1.2

# Signing is macOS's: the grant follows the signature. Elsewhere there is
# nothing to sign.
define sign
	@if command -v codesign >/dev/null; then \
		codesign --force --sign "$(SIGN_ID)" --identifier $(ENGINE_ID) engine/target/release/remuxd 2>/dev/null \
			|| echo "remuxd: not signed; run \`make remuxd.identity\` or screen recording will be asked for on every build"; \
		codesign --force --sign "$(SIGN_ID)" --identifier com.remux.cli engine/target/release/remux 2>/dev/null || true; \
	fi
endef

remuxd.build: ## Build the engine (this OS's motor) and sign it so its permissions survive a rebuild
	@$(CARGO) build --locked --release -p remuxd -p remux 
	$(sign)

# The gate. --locked everywhere: if Cargo.lock would have to change to build,
# that is a surprise and the build should say so rather than resolve around it.

obs.fetch: ## macOS: fetch OBS $(OBS_VERSION) into engine/target/obs, the copy the obs motor links and runs against (Linux: OBS from the distribution)
	@test "$(HOST_OS)" = Darwin || { echo "obs.fetch is macOS's; here: sudo apt-get install obs-studio libobs-dev"; exit 0; }
	@mkdir -p engine/target/obs && cd engine/target/obs && \
	test -f OBS-$(OBS_VERSION).dmg || curl -sL -o OBS-$(OBS_VERSION).dmg https://github.com/obsproject/obs-studio/releases/download/$(OBS_VERSION)/OBS-Studio-$(OBS_VERSION)-macOS-$$(test "$$(uname -m)" = arm64 && echo Apple || echo Intel).dmg && \
	hdiutil attach -nobrowse -quiet -mountpoint /tmp/remux-obs-dmg OBS-$(OBS_VERSION).dmg && rm -rf OBS.app && cp -R /tmp/remux-obs-dmg/OBS.app OBS.app; hdiutil detach -quiet /tmp/remux-obs-dmg; \
	echo "obs: $$(defaults read $(OBS_APP)/Contents/Info.plist CFBundleShortVersionString) in engine/target/obs"

remuxd.build.obs: ## Build the engine with the libobs motor alone (macOS: against engine/target/obs/OBS.app), signed the same way
	@test "$(HOST_OS)" != Darwin || test -d "$(OBS_APP)" || { echo "no OBS at $(OBS_APP): make obs.fetch"; exit 1; }
	@rm -f engine/target/release/obs-ffmpeg-mux engine/target/Frameworks
	@$(CARGO) build --locked --release -p remuxd -p remux --features remuxd/obs
	$(sign)

remuxd.check: ## The gate: remuxd.lint, remuxd.tests, remuxd.cover
	@$(MAKE) remuxd.lint
	@$(MAKE) remuxd.tests
	@$(MAKE) remuxd.cover

# The gate's parts, which CI calls by name so a job runs what a person runs.
# motor-obs is a workspace of its own (it links the machine's OBS), so
# `--all` never reaches it: it is formatted, linted and tested by its manifest.
remuxd.lint: ## Seam, format and clippy as errors, the engine's workspace and motor-obs's
	@$(MAKE) remuxd.seam
	@$(CARGO) fmt --all -- --check
	@$(CARGO) fmt --manifest-path motor-obs/Cargo.toml -- --check
	@$(CARGO) clippy --locked --all-targets --all-features -- -D warnings
	@$(CARGO) clippy --locked --manifest-path motor-obs/Cargo.toml --all-targets -- -D warnings

remuxd.tests: ## Every test, the engine's workspace and motor-obs's
	@$(CARGO) nextest run --locked --all-targets
	@$(CARGO) test --locked --manifest-path motor-obs/Cargo.toml

# The seam, asserted rather than trusted. remuxd-domain holds every decision
# and must never learn what an Apple framework is; keeping that true is the
# reason its coverage number means anything and the reason an objc2 upgrade
# touches a handful of files instead of the whole engine.
remuxd.seam: ## Prove the domain crate cannot see an Apple framework
	@tree=$$(cd engine && cargo tree -p remuxd-domain 2>&1) || { echo "cargo tree failed:"; echo "$$tree"; exit 1; }
	@tree=$$(cd engine && cargo tree -p remuxd-domain 2>/dev/null); \
	case "$$tree" in \
		remuxd-domain*) ;; \
		*) echo "cargo tree did not describe remuxd-domain; the check would pass on nothing"; exit 1 ;; \
	esac; \
	apple=$$(echo "$$tree" | grep -E 'objc2|block2|dispatch2' || true); \
	if [ -n "$$apple" ]; then \
		echo "remuxd-domain has grown an Apple dependency:"; echo "$$apple"; \
		echo "the domain decides; the machine is a motor. Move it there."; \
		exit 1; \
	fi; \
	echo "seam: remuxd-domain depends on $$(echo "$$tree" | tail -n +2 | grep -c '^[├└]') crates directly, none of them Apple"
	@# The daemon (the libobs motor) is the one another OS builds: no Apple crate in it either.
	@tree=$$(cd engine && cargo tree -p remuxd --no-default-features --features obs 2>/dev/null); \
	case "$$tree" in remuxd*) ;; *) echo "cargo tree did not describe remuxd"; exit 1 ;; esac; \
	apple=$$(echo "$$tree" | grep -E 'objc2|block2|dispatch2' || true); \
	if [ -n "$$apple" ]; then \
		echo "remuxd has grown an Apple dependency:"; echo "$$apple"; \
		echo "the daemon is the socket and the wiring; the machine is a motor. Move it there."; \
		exit 1; \
	fi; \
	echo "seam: remuxd sees no Apple crate"
	@# The filters are adapters the daemon injects: the domain and every face
	@# that rests on it compile none of them, nor what they bring (naga).
	@for crate in remuxd-domain remux; do \
		tree=$$(cd engine && cargo tree -p $$crate -e normal --prefix none 2>/dev/null); \
		case "$$tree" in $$crate*) ;; *) echo "cargo tree did not describe $$crate"; exit 1 ;; esac; \
		leak=$$(echo "$$tree" | grep -E '^(remux-shader|remux-mixer|naga) ' | sort -u || true); \
		if [ -n "$$leak" ]; then \
			echo "$$crate reaches a filter adapter:"; echo "$$leak"; \
			echo "the domain owns the contract; the daemon injects the adapter (remuxd/src/main.rs)."; \
			exit 1; \
		fi; \
	done; \
	echo "seam: the domain and the CLI reach no filter adapter"
	@# The golden rule: shared code never names an OS. Two tables may: the
	@# domain's (paths, the service) and the libobs motor's (the machine).
	@# `cfg(unix)` around std's file modes (0600) is std's own shim, not an OS.
	@# The working tree, not the index: a new file is the one most likely to slip.
	@named=$$(grep -rlE 'target_os|cfg\(windows' engine/domain/src engine/mixer/src engine/remuxd/src engine/wire engine/cli/src engine/motor-obs/src \
		| grep -v -e '^engine/domain/src/os.rs$$' -e '^engine/motor-obs/src/platform.rs$$' || true); \
	if [ -n "$$named" ]; then \
		echo "an OS is named outside the tables:"; echo "$$named"; \
		echo "shared code reads remuxd_domain::os::OS or motor_obs::platform::TABLE; put the difference in the table."; \
		exit 1; \
	fi; \
	tables=$$(grep -l target_os engine/domain/src/os.rs engine/motor-obs/src/platform.rs | wc -l | tr -d ' '); \
	[ "$$tables" = "2" ] || { echo "the OS tables do not name an OS; the check would pass on nothing"; exit 1; }; \
	echo "seam: no OS named outside domain/src/os.rs and motor-obs/src/platform.rs"
	@# And no path of one OS in code, comments aside: /Applications, ~/Library, /opt/homebrew, /usr/lib, ~/Movies, launchd, systemd.
	@paths=$$(grep -rnE '"[^"]*(/Applications|/System/Library|Library/Application Support|LaunchAgents|/opt/homebrew|/usr/local|/usr/lib|/usr/share|Movies/|Videos/|\.local/state|systemd|launchctl|systemctl|xdg-open)[^"]*"' \
		engine/domain/src engine/mixer/src engine/remuxd/src engine/wire engine/cli/src engine/motor-obs/src \
		| grep -v -e '^engine/domain/src/os.rs:' -e '^engine/motor-obs/src/platform.rs:' -e '^[^:]*:[0-9]*:[[:space:]]*//' -e 'assert' || true); \
	if [ -n "$$paths" ]; then \
		echo "a path of one OS is written outside the tables:"; echo "$$paths"; exit 1; \
	fi; \
	echo "seam: no path of one OS outside the tables"

remuxd.test: ## Tests, mid-loop. F=name to filter
	@$(CARGO) nextest run --locked -p remuxd-domain -p remux-mixer -p remuxd -p remux $(F)

# Coverage is measured on the domain crate alone, which is now a crate boundary
# rather than a path regex. The daemon's own files are transport and adapters:
# they cannot run without a display and a TCC grant, they are proven by
# `make `, and counting them would only ever produce a number low
# enough to be ignored. Same principle as mix.exs ignoring Application and Repo.
#
# Lines and functions, not branches: Rust emits branch counters only on nightly.
# The CLI's words are decisions and counted; its socket and its service
# (main.rs, daemon.rs) are transport, left out the way the motors are.
remuxd.cover: ## Coverage of the decisions, and fail under the gate
	@$(CARGO) llvm-cov nextest --locked -p remuxd-domain -p remux-mixer -p remux --summary-only \
		--ignore-filename-regex 'cli/src/(main|daemon)\.rs' \
		--fail-under-lines 90 --fail-under-functions 90

# The engine, configured, in the foreground: everything it needs comes from
# one place, so the last step of anything is one command rather than an
# environment somebody assembles from memory.
# The engine's environment, assembled once for the foreground and the
# background: the music folder (recordings go where the OS table says). Where a live goes
# is the destinations file's; `REMUXD_RTMP` overrides that when set (a test
# live, a file). The operator's own folders win when
# ~/.config/remux/operator.env exists (0600, KEY=value lines).
define remuxd-env
export REMUX_MUSIC_DIR="$${REMUX_MUSIC_HOST:-./music}"; \
if [ -f "$$HOME/.config/remux/operator.env" ]; then set -a; . "$$HOME/.config/remux/operator.env"; set +a; fi
endef

remuxd.run: remuxd.build ## Run the engine in the foreground
	@$(remuxd-env); \
	echo "remuxd: publishing to the destinations kept, recording where remux config says"; \
	echo "        music from $$REMUX_MUSIC_DIR"; \
	echo "        drive it with \`remux status\`"; \
	./engine/target/release/remuxd

# The engine in the background, for a face to stand on. Idempotent: an
# engine already listening is left alone. Its words go to its own log,
# beside the socket, which is what a face reads when it says "No engine".
# Stopping it is a face's lease running out or `remux quit`.
remuxd.start: remuxd.build ## Start the engine in the background if none is listening; a face's lease or `remux quit` stops it
	@if ./engine/target/release/remux status >/dev/null 2>&1; then \
		echo "remuxd: already listening"; exit 0; \
	fi; \
	$(remuxd-env); \
	nohup ./engine/target/release/remuxd >/dev/null 2>&1 & \
	for i in $$(seq 1 60); do \
		./engine/target/release/remux status >/dev/null 2>&1 && break; sleep 0.5; \
	done; \
	./engine/target/release/remux status >/dev/null 2>&1 \
		|| { echo "remuxd did not answer in 30s; its log: remux daemon log"; exit 1; }; \
	echo "remuxd: listening; its log: remux daemon log"

