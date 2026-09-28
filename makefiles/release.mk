##@ Release (the public build: the CLI and the engine on the libobs motor; a tarball a dev installs with install.sh.)
.PHONY: release release.install release.uninstall release.publish

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

# The GitHub release of this version: the tag is the version, the notes are
# docs/releases/<version>.md, the assets are what dist/ holds (this machine's
# tarball, plus any other target's dropped in). A tag that already exists is
# not moved; a release that already exists gets the assets added.
release.publish: ## Tag v$(VERSION), create the GitHub release with docs/releases/$(VERSION).md and upload dist/
	@test -f "$(DIST)/$(RELEASE).tar.gz.sha256" || { echo "no dist: make release"; exit 1; }
	@test -f "docs/releases/$(VERSION).md" || { echo "no notes: write docs/releases/$(VERSION).md"; exit 1; }
	@test -z "$$(git status --porcelain)" || { echo "the tree is not clean; commit first"; exit 1; }
	@git rev-parse "v$(VERSION)" >/dev/null 2>&1 || git tag -a "v$(VERSION)" -m "remux $(VERSION)"
	@git push -q origin "v$(VERSION)"
	@if gh release view "v$(VERSION)" >/dev/null 2>&1; then gh release upload "v$(VERSION)" $(DIST)/*.tar.gz $(DIST)/*.sha256 --clobber; \
	else gh release create "v$(VERSION)" $(DIST)/*.tar.gz $(DIST)/*.sha256 --title "remux $(VERSION)" --notes-file "docs/releases/$(VERSION).md"; fi
	@echo "https://github.com/remuxhq/core/releases/tag/v$(VERSION)"
