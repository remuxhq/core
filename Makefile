SHELL = /bin/bash
.DEFAULT_GOAL := help

# Cargo runs from the workspace, so the pinned toolchain and nextest's config
# are found; every cargo recipe goes through this.
CARGO := cd engine && cargo

# The certificate the engine is signed with on macOS. macOS ties a screen
# recording grant to the code signature; ad hoc there is no certificate to
# anchor to, so the tie is the binary's own hash and every rebuild is a
# stranger. `make remuxd.identity` creates it once, in the login keychain.
SIGN_ID := remux dev

.PHONY: help cli.link

help: ## Show every target, grouped by context
	@awk 'BEGIN {FS = ":.*##"} /^##@/ {printf "\n%s\n", substr($$0, 5)} /^[a-zA-Z0-9_.-]+:.*?##/ {printf "  %-22s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

##@ The shell
cli.link: ## `remux` on the PATH: a link to the built CLI in ~/.local/bin (where install.sh puts it)
	@mkdir -p "$(HOME)/.local/bin" && ln -sf "$(CURDIR)/engine/target/release/remux" "$(HOME)/.local/bin/remux" \
		&& echo "remux -> $(HOME)/.local/bin/remux (on your PATH?)"

include makefiles/remuxd.mk
include makefiles/music.mk
include makefiles/security.mk
include makefiles/release.mk
