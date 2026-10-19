.PHONY: all build frontend test run clean

all: build

frontend:
	cd frontend && npm install && npm run build

build: frontend
	cargo build --release

test:
	cargo test

# The binary embeds frontend/dist, so build it first: a fresh clone has none
run: frontend
	cargo run -- serve

clean:
	cargo clean
	rm -rf frontend/dist
