.PHONY: all build frontend test run clean

all: build

frontend:
	cd frontend && npm install && npm run build

build: frontend
	cargo build --release

test:
	cargo test

run:
	cargo run -- serve

clean:
	cargo clean
	rm -rf frontend/dist
