# failBrauwser build entry points. `make` builds everything into target/.
CWB_ROOT ?= $(CURDIR)/vendor/compressionworkbench
HELPER_OUT := target/helper
# Extra `dotnet publish` options, e.g. -p:CppCompilerAndLinker=gcc where clang is missing.
HELPER_FLAGS ?=

MODULE_OUT := target/module

.PHONY: all app helper module test clean

all: app helper module

app:
	cargo build --release

helper:
	dotnet publish helper/FbArchive/FbArchive.csproj -c Release -r linux-x64 \
		"-p:CwbRoot=$(CWB_ROOT)" $(HELPER_FLAGS) -o $(HELPER_OUT) --nologo -v quiet
	rm -f $(HELPER_OUT)/*.pdb $(HELPER_OUT)/*.xml $(HELPER_OUT)/*.dbg

# The GTK 3 module giving GTK 3 applications failBrauwser's open and save dialogs.
module:
	mkdir -p $(MODULE_OUT)
	$(CC) -shared -fPIC -O2 -Wall $$(pkg-config --cflags gtk+-3.0) -o $(MODULE_OUT)/libfailbrauwser-chooser.so \
		gtkmodule/failbrauwser-chooser.c $$(pkg-config --libs gtk+-3.0)

test: helper
	dotnet test helper/FbArchive.Tests/FbArchive.Tests.csproj "-p:CwbRoot=$(CWB_ROOT)" --nologo -v quiet
	cargo test

clean:
	cargo clean
	rm -rf helper/FbArchive/bin helper/FbArchive/obj
