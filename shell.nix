{ pkgs ? import <nixpkgs> { }, ... }: pkgs.mkShell {
  buildInputs = with pkgs; [
    wayland
    wayland-protocols
    cmake
    fontconfig
    openssl
    pkg-config
    xorg.libX11
    libGL
    libxkbcommon
    xorg.libX11
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
    alsa-lib
  ];
  LD_LIBRARY_PATH = with pkgs; lib.makeLibraryPath [
    libGL
    libxkbcommon
    wayland
    xorg.libX11
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
    alsa-lib
  ];
}
