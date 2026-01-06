.PHONY: init run test icon

UNAME_S := $(shell uname -s)
CMAKE ?= cmake
BUILD_DIR ?= build
BUILD_TYPE ?= Debug

ifeq ($(UNAME_S),Darwin)
QT_PREFIX ?= /opt/homebrew/opt/qt
endif

CMAKE_PREFIX_ARG := $(if $(QT_PREFIX),-DCMAKE_PREFIX_PATH=$(QT_PREFIX),)

init:
	@mkdir -p $(BUILD_DIR)
	@if [ "$(UNAME_S)" = "Darwin" ]; then \
		if ! command -v brew >/dev/null 2>&1; then \
			echo "Homebrew not found. Install it from https://brew.sh/"; \
			exit 1; \
		fi; \
		brew list qt >/dev/null 2>&1 || brew install qt; \
		brew list opencascade >/dev/null 2>&1 || brew install opencascade; \
		brew list eigen >/dev/null 2>&1 || brew install eigen; \
	else \
		echo "Please install Qt6, OpenCASCADE, and Eigen3 for your platform."; \
	fi
	@$(CMAKE) -S . -B $(BUILD_DIR) -DCMAKE_BUILD_TYPE=$(BUILD_TYPE) $(CMAKE_PREFIX_ARG)

run:
	@$(CMAKE) --build $(BUILD_DIR)
	@if [ -x "$(BUILD_DIR)/OneCAD" ]; then \
		"$(BUILD_DIR)/OneCAD"; \
	else \
		"$(BUILD_DIR)/OneCAD.app/Contents/MacOS/OneCAD"; \
	fi

test:
	@$(CMAKE) --build $(BUILD_DIR) --target proto_custom_map
	@$(CMAKE) --build $(BUILD_DIR) --target proto_tnaming
	@$(CMAKE) --build $(BUILD_DIR) --target proto_elementmap_rigorous
	@$(BUILD_DIR)/tests/proto_custom_map
	@$(BUILD_DIR)/tests/proto_tnaming
	@$(BUILD_DIR)/tests/proto_elementmap_rigorous

ICON_SRC ?= icon.png
ICON_OUT := resources/AppIcon.icns
ICONSET := resources/AppIcon.iconset

icon:
	@if [ ! -f "$(ICON_SRC)" ]; then echo "Error: $(ICON_SRC) not found"; exit 1; fi
	@echo "Generating $(ICON_OUT) from $(ICON_SRC)"
	@mkdir -p $(ICONSET)
	@sips -z 16 16 $(ICON_SRC) --out $(ICONSET)/icon_16x16.png >/dev/null
	@sips -z 32 32 $(ICON_SRC) --out $(ICONSET)/icon_16x16@2x.png >/dev/null
	@sips -z 32 32 $(ICON_SRC) --out $(ICONSET)/icon_32x32.png >/dev/null
	@sips -z 64 64 $(ICON_SRC) --out $(ICONSET)/icon_32x32@2x.png >/dev/null
	@sips -z 128 128 $(ICON_SRC) --out $(ICONSET)/icon_128x128.png >/dev/null
	@sips -z 256 256 $(ICON_SRC) --out $(ICONSET)/icon_128x128@2x.png >/dev/null
	@sips -z 256 256 $(ICON_SRC) --out $(ICONSET)/icon_256x256.png >/dev/null
	@sips -z 512 512 $(ICON_SRC) --out $(ICONSET)/icon_256x256@2x.png >/dev/null
	@sips -z 512 512 $(ICON_SRC) --out $(ICONSET)/icon_512x512.png >/dev/null
	@sips -z 1024 1024 $(ICON_SRC) --out $(ICONSET)/icon_512x512@2x.png >/dev/null
	@iconutil -c icns $(ICONSET) -o $(ICON_OUT)
	@rm -rf $(ICONSET)
	@echo "Done: $(ICON_OUT)"
