# Built by packaging/scripts/build-rpm.sh from a payload staged by stage-payload.sh.
# Defines expected from the caller: _cmux_version, _cmux_payload (staged file tree).

Name:           cmux
Version:        %{_cmux_version}
Release:        1%{?dist}
Summary:        GPU-accelerated terminal multiplexer
License:        AGPL-3.0-or-later
URL:            https://github.com/TNortnern/cmux-linux
BuildArch:      x86_64

# Prebuilt binaries; nothing is compiled here. Dependencies are listed explicitly
# because the binaries are not scanned for automatic requirements.
AutoReqProv:    no
%global debug_package %{nil}
# Do not create /usr/lib/.build-id links for the prebuilt binaries
%define _build_id_links none

Requires:       gtk4 >= 4.6
Requires:       fontconfig
Requires:       freetype
Requires:       mesa-libGL
Requires:       bzip2-libs
Requires:       libstdc++
Requires:       libgcc

%description
cmux is a GPU-accelerated terminal multiplexer for Linux with workspaces, tabs,
splits, session restore, notifications and socket CLI control, powered by Ghostty.
Browser automation (agent-browser) and SSH workspaces are optional.

%install
cp -a %{_cmux_payload}/. %{buildroot}/

%files
%license /usr/share/licenses/cmux/LICENSE
%{_bindir}/cmux-app
%{_bindir}/cmux-app.bin
%{_bindir}/cmux
/usr/lib/cmux
%{_datadir}/applications/com.cmux_lx.terminal.desktop
%{_datadir}/metainfo/com.cmux_lx.terminal.metainfo.xml
%{_datadir}/icons/hicolor/48x48/apps/com.cmux_lx.terminal.png
%{_datadir}/icons/hicolor/128x128/apps/com.cmux_lx.terminal.png
%{_datadir}/icons/hicolor/256x256/apps/com.cmux_lx.terminal.png
%{_datadir}/icons/hicolor/scalable/apps/com.cmux_lx.terminal.svg
%{_datadir}/bash-completion/completions/cmux
%{_datadir}/zsh/vendor-completions/_cmux
%{_datadir}/fish/vendor_completions.d/cmux.fish
%{_mandir}/man1/cmux.1.gz
%{_datadir}/cmux

%changelog
* Mon Oct 19 2026 cmux-linux contributors - 0.2.0-1
- Release package for cmux-linux 0.2.0
