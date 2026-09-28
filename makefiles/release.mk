##@ Release (the public build: the CLI and the engine on the libobs motor; a tarball a dev installs with install.sh.)
.PHONY: release release.install release.uninstall

VERSION  := $(shell sed -n 's/^version = "\(.*\)"/\1/p' engine/cli/Cargo.toml | head -1)
TARGET   := $(shell uname -m | sed s/arm64/aarch64/)-$(shell uname -s | tr A-Z a-z | sed "s/darwin/apple-darwin/; s/linux/unknown-linux-gnu/")
RELEASE  := remux-$(VERSION)-$(TARGET)
DIST     := $(CURDIR)/dist
STAGE    := $(DIST)/$(RELEASE)

release: ## Build dist/$(RELEASE).tar.gz for this machine (scripts/release.sh; CI runs it per target)
	@OBS_APP="$(OBS_APP)" SIGN_ID="$(SIGN_ID)" sh scripts/release.sh "$(DIST)"

release.install: ## Install dist/ the way a dev would from GitHub (curl | sh, sha256, ~/.local), then start the daemon
	@test -f "$(DIST)/$(RELEASE).tar.gz.sha256" || { echo "no dist: make release"; exit 1; }
	@env -u OBS_APP REMUX_RELEASE_URL="file://$(DIST)" REMUX_VERSION="$(VERSION)" sh install.sh -y

release.uninstall: ## Remove what install.sh put on this machine (config kept: PURGE=1 removes it too)
	@sh uninstall.sh $(if $(PURGE),--purge,)
