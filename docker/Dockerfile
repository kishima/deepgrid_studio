# gaia-maker build & run environment
# Bevy 0.15 / Rust edition 2024 (needs Rust >= 1.85)
#
# Single image that can both COMPILE the project and RUN the GUI binary,
# so the produced binary's glibc always matches its runtime (no host glibc
# mismatch). GUI is shown on the Windows desktop through WSLg.
FROM rust:1-bookworm

# --- Bevy build deps + GUI/audio runtime libs (X11 + Wayland + Mesa/Vulkan) ---
RUN apt-get update && apt-get install -y --no-install-recommends \
    # build
    pkg-config \
    g++ \
    libasound2-dev \
    libudev-dev \
    # X11 windowing (bevy "x11" feature)
    libx11-6 \
    libxcursor1 \
    libxrandr2 \
    libxi6 \
    libxkbcommon0 \
    libxkbcommon-x11-0 \
    # Wayland windowing
    libwayland-client0 \
    # GPU / software rendering stack for wgpu (GL + Vulkan)
    libgl1-mesa-dri \
    libegl1 \
    libgbm1 \
    libgles2 \
    libvulkan1 \
    mesa-vulkan-drivers \
    vulkan-tools \
    ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Source tree is bind-mounted at /app at run time; target/ lands back on the host.
WORKDIR /app
CMD ["bash"]
