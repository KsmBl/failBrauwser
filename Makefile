# failBrauwser build entry points. `make` builds everything into target/.
CWB_ROOT ?= $(abspath ../compressionworkbench)
HELPER_OUT := target/helper

.PHONY: all app helper test clean

all: app helper

app:
	cargo build --release

helper:
	dotnet publish helper/FbArchive/FbArchive.csproj -c Release -r linux-x64 \
		-p:CwbRoot=$(CWB_ROOT) -o $(HELPER_OUT) --nologo -v quiet
	rm -f $(HELPER_OUT)/*.pdb $(HELPER_OUT)/*.xml $(HELPER_OUT)/*.dbg

test: helper
	cargo test

clean:
	cargo clean
	rm -rf helper/FbArchive/bin helper/FbArchive/obj
