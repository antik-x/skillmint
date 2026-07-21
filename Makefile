.PHONY: build install build-install clean verify verify-bundle

PROJECT_ROOT := $(shell pwd)
APP_NAME := SkillMint
APP_BUNDLE := $(PROJECT_ROOT)/skillmint/src-tauri/target/release/bundle/macos/$(APP_NAME).app
DMG_BUNDLE := $(PROJECT_ROOT)/skillmint/src-tauri/target/release/bundle/dmg/$(APP_NAME)_0.1.0_aarch64.dmg
INSTALL_PATH := /Applications/$(APP_NAME).app
NVM_CMD := export NVM_DIR="$(HOME)/.nvm"; [ -s "$(NVM_DIR)/nvm.sh" ] && . "$(NVM_DIR)/nvm.sh"; nvm use 22
TARGET_TRIPLE := $(shell rustc -vV | sed -n 's/^host: //p')

# Bundle content assertions, parameterized on the .app path. P2-2-a: tray
# dual-state icons and the repair_paths sidecar must ship in the bundle.
define assert_bundle
	@if [ ! -f "$(1)/Contents/MacOS/skillmint" ]; then \
		echo "ERROR: Main executable 'skillmint' not found in $(1)"; \
		exit 1; \
	fi
	@if [ "$$(plutil -extract CFBundleExecutable raw '$(1)/Contents/Info.plist')" != "skillmint" ]; then \
		echo "ERROR: CFBundleExecutable does not point to 'skillmint'"; \
		exit 1; \
	fi
	@for f in tray-normal.png tray-warning.png; do \
		if [ ! -f "$(1)/Contents/Resources/$$f" ]; then \
			echo "ERROR: $$f missing from $(1)/Contents/Resources"; \
			exit 1; \
		fi; \
	done
	@if [ ! -f "$(1)/Contents/MacOS/repair_paths" ]; then \
		echo "ERROR: repair_paths sidecar not found in $(1)/Contents/MacOS"; \
		exit 1; \
	fi
endef

build-install: build install verify

build:
	@echo "==> Building SkillMint Tauri app..."
	@# P2-2-a: stage the repair_paths sidecar before bundling so
	@# externalBin can pack it into Contents/MacOS. tauri-build validates
	@# externalBin paths during EVERY cargo build, so the sidecar path must
	@# exist up-front (placeholder on a fresh checkout; always overwritten
	@# with the real artifact below).
	mkdir -p $(PROJECT_ROOT)/skillmint/src-tauri/bin
	@if [ -f "$(PROJECT_ROOT)/skillmint/src-tauri/target/release/repair_paths" ]; then \
		cp "$(PROJECT_ROOT)/skillmint/src-tauri/target/release/repair_paths" \
			"$(PROJECT_ROOT)/skillmint/src-tauri/bin/repair_paths-$(TARGET_TRIPLE)"; \
	else \
		touch "$(PROJECT_ROOT)/skillmint/src-tauri/bin/repair_paths-$(TARGET_TRIPLE)"; \
	fi
	cd $(PROJECT_ROOT)/skillmint/src-tauri && cargo build --release --features custom-protocol --bin repair_paths
	cp $(PROJECT_ROOT)/skillmint/src-tauri/target/release/repair_paths \
		$(PROJECT_ROOT)/skillmint/src-tauri/bin/repair_paths-$(TARGET_TRIPLE)
	cd $(PROJECT_ROOT)/skillmint && $(NVM_CMD) && npm run tauri build

install:
	@echo "==> Installing $(APP_NAME).app to /Applications..."
	@if [ -d "$(INSTALL_PATH)" ]; then \
		rm -rf "$(INSTALL_PATH)"; \
	fi
	cp -R "$(APP_BUNDLE)" /Applications/
	@echo "==> Installed to $(INSTALL_PATH)"

verify-bundle:
	@echo "==> Verifying built bundle..."
	$(call assert_bundle,$(APP_BUNDLE))
	@echo "==> Bundle verification passed."

verify:
	@echo "==> Verifying installation..."
	$(call assert_bundle,$(INSTALL_PATH))
	@echo "==> Verification passed. Main executable: $$(stat -f '%Sm' '$(INSTALL_PATH)/Contents/MacOS/skillmint')"

clean:
	@echo "==> Cleaning Tauri build artifacts..."
	rm -rf $(PROJECT_ROOT)/skillmint/src-tauri/target/release/bundle

clean:
	@echo "==> Cleaning Tauri build artifacts..."
	rm -rf $(PROJECT_ROOT)/skillmint/src-tauri/target/release/bundle
