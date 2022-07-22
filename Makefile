EXAMPLE_NAME := fido

all: | start-sim attach finish-message

.PHONY: finish-message
finish-message:
	@echo "###################################################"
	@echo "Done. Device should be visible in your system now. Run 'make stop' to disconnect it."

.PHONY: start-sim
start-sim: build
	-$(MAKE) stop
	env RUST_LOG=debug cargo run --example $(EXAMPLE_NAME) &
	sleep 1

.PHONY: attach
attach: 
	lsmod | grep vhci-hcd || sudo modprobe vhci-hcd
	sudo usbip list -r "localhost"
	sudo usbip attach -r "localhost" -b "1-1"
	sudo usbip attach -r "localhost" -b "1-1"
	sleep 5

.PHONY: ci
ci:
	timeout 10 -k 5 $(MAKE)

.PHONY: build
build:
	 cargo build --example $(EXAMPLE_NAME)

.PHONY: build-clean
build-clean: | clean build

.PHONY: stop
stop:
	-sudo usbip detach -p "00"
	killall $(EXAMPLE_NAME)

.PHONY: setup-fedora
setup-fedora:
	sudo dnf install usbip clang-libs-13.0.0
	sudo ln -s /usr/lib64/libclang.so.13 /usr/lib64/libclang.so

.PHONY: clean
clean:
	cargo clean
	rm -r cargo-cache

.PHONY: build-docker
CMD=make -C /app/ build
build-docker:
	docker build -t usbip .
	mkdir -p cargo-cache
	docker run -it --rm -v $(PWD)/cargo-cache:/root/.cargo -v $(PWD):/app usbip $(CMD)
