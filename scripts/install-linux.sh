#!/usr/bin/env bash
set -euo pipefail
bundle=$(cd "$(dirname "$0")" && pwd)
prefix=${1:-"$HOME/.local"}
install -Dm755 "$bundle/bin/queri" "$prefix/bin/queri"
install -Dm644 "$bundle/share/icons/hicolor/scalable/apps/dev.queri.Queri.svg" "$prefix/share/icons/hicolor/scalable/apps/dev.queri.Queri.svg"
install -d "$prefix/share/applications"
python3 - "$bundle/share/applications/dev.queri.Queri.desktop" "$prefix" <<'PY'
import pathlib, sys
source, prefix = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]).resolve()
binary = str(prefix / 'bin' / 'queri')
# Desktop Entry command arguments have their own escaping rules.
escaped = binary.replace('\\', '\\\\\\\\').replace('"', '\\\\"').replace('`', '\\\\`').replace('$', '\\\\$').replace('%', '%%')
text = source.read_text().replace('Exec=queri', f'Exec="{escaped}"')
(prefix / 'share/applications/dev.queri.Queri.desktop').write_text(text)
PY
if command -v update-desktop-database >/dev/null; then
  update-desktop-database "$prefix/share/applications"
fi
echo "Installed $prefix/bin/queri"
