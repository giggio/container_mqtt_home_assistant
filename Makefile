.PHONY: default build test clean run build_release docker_build_amd64_static docker_build_arm64_static docker_build_all docker_build_multiarch release

NO_PUSH ?= 0
amd64_target := x86_64
arm64_target := aarch64
binary := cmha
image_name := cmha
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

docker_build_amd64_static: target/tmp/$(binary)_$(amd64_target)
	docker buildx build -f Containerfile --build-arg target_file=target/tmp/$(binary)_$(amd64_target) \
		-t giggio/$(image_name):$(version)-amd64 -t giggio/$(image_name):amd64 --platform linux/amd64 --build-arg PLATFORM=x86_64 $(if $(NO_PUSH),, --push) .

docker_build_arm64_static: target/tmp/$(binary)_$(arm64_target)
	docker buildx build -f Containerfile --build-arg target_file=target/tmp/$(binary)_$(arm64_target) \
		-t giggio/$(image_name):$(version)-arm64 -t giggio/$(image_name):arm64 --platform linux/arm64 --build-arg PLATFORM=aarch64 $(if $(NO_PUSH),, --push) .

docker_build_all: docker_build_amd64_static docker_build_arm64_static

docker_build_multiarch:
	docker buildx imagetools create -t giggio/$(image_name):latest \
		giggio/$(image_name):amd64 \
		giggio/$(image_name):arm64
	docker buildx imagetools create -t giggio/$(image_name):$(version) \
		giggio/$(image_name):$(version)-amd64 \
		giggio/$(image_name):$(version)-arm64

release: docker_build_all docker_build_multiarch
