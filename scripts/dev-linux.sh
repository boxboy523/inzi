#!/bin/sh
set -eu

export GDK_BACKEND=x11
export WEBKIT_DISABLE_DMABUF_RENDERER=1
export WEBKIT_DISABLE_COMPOSITING_MODE=1

# Nix development shells may not discover the system EGL vendor manifests.
if [ -z "${__EGL_VENDOR_LIBRARY_FILENAMES:-}" ]; then
    inzi_egl_vendors=""
    for inzi_manifest in /run/opengl-driver/share/glvnd/egl_vendor.d/*.json /etc/glvnd/egl_vendor.d/*.json /usr/share/glvnd/egl_vendor.d/*.json; do
        if [ -f "$inzi_manifest" ]; then
            inzi_egl_vendors="${inzi_egl_vendors:+$inzi_egl_vendors:}$inzi_manifest"
        fi
    done
    if [ -n "$inzi_egl_vendors" ]; then
        export __EGL_VENDOR_LIBRARY_FILENAMES="$inzi_egl_vendors"
    fi
fi

if [ -d /run/opengl-driver/lib ]; then
    export LD_LIBRARY_PATH="/run/opengl-driver/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

echo "[Linux preview] EGL vendors: ${__EGL_VENDOR_LIBRARY_FILENAMES:-automatic}"
exec pnpm tauri dev --target x86_64-unknown-linux-gnu "$@"
