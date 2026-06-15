#!/bin/bash
# Install system development packages required to build GuiShell
set -e

echo "Installing GTK4, libadwaita, and other dev packages..."
sudo apt install -y \
  libgtk-4-dev libadwaita-1-dev \
  libsecret-1-dev libssh2-1-dev libcairo2-dev \
  pkg-config build-essential

echo ""
echo "Verifying installed packages..."
echo "  gtk4:        $(pkg-config --modversion gtk4)"
echo "  libadwaita:  $(pkg-config --modversion libadwaita-1)"
echo "  cairo:       $(pkg-config --modversion cairo)"

# VTE GTK4 is not available on Ubuntu 22.04 standard repos.
# The vte4 crate is an optional dependency.
# To enable VTE support, you need to install libvte-2.91-gtk4-dev
# from a PPA or build VTE from source with GTK4 support.
echo ""
echo "NOTE: VTE GTK4 (libvte-2.91-gtk4-dev) is not available on Ubuntu 22.04."
echo "      The vte4 dependency is optional. Build with: cargo build --features vte"
echo "      once VTE GTK4 is installed."

touch /tmp/deps_installed
echo ""
echo "Done! You can now run: cargo build"
