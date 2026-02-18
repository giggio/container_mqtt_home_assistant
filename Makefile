.PHONY: default build test clean run build_release buildah_build_amd64_static buildah_build_arm64_static buildah_build_all buildah_build_multiarch release

NO_PUSH :=
amd64_target := x86_64
arm64_target := aarch64
binary := cmha
image_name := docker.io/library/giggio/cmha
version := $(shell yq -oy .package.version cmha/Cargo.toml)
rust_deps := $(shell git ls-files --cached --modified --others --exclude-standard '*.rs' ':**/Cargo.toml' ':**/Cargo.lock')

default: release

build:
	cargo build

test:
	cargo nextest run

clean:
	cargo clean

run:
	cargo run

build_release:
	cargo build --release

target/tmp/$(binary)_$(amd64_target): $(rust_deps)
	nix build .\#$(amd64_target) --print-build-logs
	mkdir -p target/tmp
	cp -f result/bin/$(binary)_$(amd64_target) target/tmp/

target/tmp/$(binary)_$(arm64_target): $(rust_deps)
	nix build .\#$(arm64_target) --print-build-logs
	mkdir -p target/tmp
	cp -f result/bin/$(binary)_$(arm64_target) target/tmp/

buildah_build_amd64_static: target/tmp/$(binary)_$(amd64_target)
	buildah build -f Containerfile --build-arg target_file=target/tmp/$(binary)_$(amd64_target) \
		-t $(image_name):$(version)-amd64 -t $(image_name):amd64 \
	  --platform linux/amd64 --signature-policy ./buildah-policy.json --format=docker --pull .
	if [ "$(NO_PUSH)" == "" ]; then \
	  buildah push $(image_name):$(version)-amd64 -t $(image_name):amd64; \
	fi

buildah_build_arm64_static: target/tmp/$(binary)_$(arm64_target)
	buildah build -f Containerfile --build-arg target_file=target/tmp/$(binary)_$(arm64_target) \
		-t $(image_name):$(version)-arm64 -t $(image_name):arm64 \
	  --platform linux/arm64 --signature-policy ./buildah-policy.json --format=docker --pull .
	if [ "$(NO_PUSH)" == "" ]; then \
	  buildah push $(image_name):$(version)-arm64 -t $(image_name):arm64; \
	fi

buildah_build_all: buildah_build_amd64_static buildah_build_arm64_static

buildah_build_multiarch:
	buildah manifest rm $(image_name):latest 2> /dev/null || true
	buildah manifest create $(image_name):latest
	buildah manifest add $(image_name):latest $(image_name):amd64
	buildah manifest add $(image_name):latest $(image_name):arm64
	if [ "$(NO_PUSH)" == "" ]; then \
    buildah manifest push --all $(image_name):latest; \
	fi
	buildah manifest rm $(image_name):$(version) 2> /dev/null || true
	buildah manifest create $(image_name):$(version)
	buildah manifest add $(image_name):$(version) $(image_name):$(version)-amd64
	buildah manifest add $(image_name):$(version) $(image_name):$(version)-arm64
	if [ "$(NO_PUSH)" == "" ]; then \
	  buildah manifest push --all $(image_name):$(version); \
	fi

release: buildah_build_all buildah_build_multiarch
