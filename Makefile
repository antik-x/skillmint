.PHONY: build install build-install clean verify

PROJECT_ROOT := $(shell pwd)
APP_NAME := SkillMint
APP_BUNDLE := $(PROJECT_ROOT)/skillmint/src-tauri/target/release/bundle/macos/$(APP_NAME).app
DMG_BUNDLE := $(PROJECT_ROOT)/skillmint/src-tauri/target/release/bundle/dmg/$(APP_NAME)_0.1.0_aarch64.dmg
INSTALL_PATH := /Applications/$(APP_NAME).app
NVM_CMD := export NVM_DIR="$(HOME)/.nvm"; [ -s "$(NVM_DIR)/nvm.sh" ] && . "$(NVM_DIR)/nvm.sh"; nvm use 22

build-install: build install verify

build:
	@echo "==> Building SkillMint Tauri app..."
	cd $(PROJECT_ROOT)/skillmint && $(NVM_CMD) && npm run tauri build

install:
	@echo "==> Installing $(APP_NAME).app to /Applications..."
	@if [ -d "$(INSTALL_PATH)" ]; then \
		rm -rf "$(INSTALL_PATH)"; \
	fi
	cp -R "$(APP_BUNDLE)" /Applications/
	@echo "==> Installed to $(INSTALL_PATH)"

verify:
	@echo "==> Verifying installation..."
	@if [ ! -f "$(INSTALL_PATH)/Contents/MacOS/skillmint" ]; then \
		echo "ERROR: Main executable 'skillmint' not found in bundle"; \
		exit 1; \
	fi
	@if [ "$$(plutil -extract CFBundleExecutable raw '$(INSTALL_PATH)/Contents/Info.plist')" != "skillmint" ]; then \
		echo "ERROR: CFBundleExecutable does not point to 'skillmint'"; \
		exit 1; \
	fi
	@echo "==> Verification passed. Main executable: $$(stat -f '%Sm' '$(INSTALL_PATH)/Contents/MacOS/skillmint')"

clean:
	@echo "==> Cleaning Tauri build artifacts..."
	rm -rf $(PROJECT_ROOT)/skillmint/src-tauri/target/release/bundle
