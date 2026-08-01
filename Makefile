.PHONY: build run install uninstall

build:
	cargo build --release

run:
	cargo run --release

install:
	sudo cp target/release/gitr /usr/local/bin/gitr
	@# Optional: install desktop entry and icon
	@if command -v xdg-desktop-menu >/dev/null 2>&1; then \
		mkdir -p $(HOME)/.local/share/applications; \
		mkdir -p $(HOME)/.local/share/icons/hicolor/scalable/apps; \
		cp packaging/gitr.svg $(HOME)/.local/share/icons/hicolor/scalable/apps/ || true; \
		cat > $(HOME)/.local/share/applications/gitr.desktop <<- EOF; \
		[Desktop Entry] \
		Type=Application \
		Name=gitr \
		Comment=Compact git commit graph viewer \
		Exec=/usr/local/bin/gitr \
		Icon=gitr \
		Categories=Development; \
		Terminal=false \
		EOF \
		xdg-desktop-menu forceupdate 2>/dev/null || true; \
	fi
	@echo "Installed. Run 'gitr' from your terminal."

uninstall:
	sudo rm -f /usr/local/bin/gitr
	rm -f $(HOME)/.local/share/applications/gitr.desktop
	rm -f $(HOME)/.local/share/icons/hicolor/scalable/apps/gitr.svg
