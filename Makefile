.PHONY: default build test clean run build_release build_amd64_static docker_build_amd64_static release_amd64_static build_arm64_static docker_build_arm64_static release_arm64_static release_with_docker_only release

amd64_target := x86_64
arm64_target := aarch64
binary := cmha
image_name := cmha
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

build_amd64_static:
	nix build .\#$(amd64_target)
	mkdir -p target/tmp/$(amd64_target)/
	cp -f result/bin/$(binary) target/tmp/$(amd64_target)/

docker_build_amd64_static:
	mkdir -p target/output
	cp -f target/tmp/$(amd64_target)/$(binary) target/output/
	VERSION=$$(yq -oy .package.version cmha/Cargo.toml); \
	docker buildx build -f Containerfile -t giggio/$(image_name):$$VERSION-amd64 -t giggio/$(image_name):amd64 --platform linux/amd64 --build-arg PLATFORM=x86_64 --push .

release_amd64_static: build_amd64_static docker_build_amd64_static

build_arm64_static:
	nix build .\#$(arm64_target)
	mkdir -p target/tmp/$(arm64_target)/
	cp -f result/bin/$(binary) target/tmp/$(arm64_target)/

docker_build_arm64_static:
	mkdir -p target/output
	cp -f target/tmp/$(arm64_target)/$(binary) target/output/
	VERSION=$$(yq -oy .package.version cmha/Cargo.toml); \
	docker buildx build -f Containerfile -t giggio/$(image_name):$$VERSION-arm64 -t giggio/$(image_name):arm64 --platform linux/arm64 --build-arg PLATFORM=aarch64 --push .

release_arm64_static: build_arm64_static docker_build_arm64_static

release_with_docker_only:
	docker buildx imagetools create -t giggio/$(image_name):latest \
		giggio/$(image_name):amd64 \
		giggio/$(image_name):arm64; \
	VERSION=$$(yq -oy .package.version cmha/Cargo.toml); \
    docker buildx imagetools create -t giggio/$(image_name):$$VERSION \
		giggio/$(image_name):$$VERSION-amd64 \
		giggio/$(image_name):$$VERSION-arm64;

release: release_amd64_static release_arm64_static release_with_docker_only
