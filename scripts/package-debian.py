#!/usr/bin/env python3
"""Package the compiled desktop app without requiring root or cargo-deb."""
import argparse
import hashlib
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument('--version')
parser.add_argument('--binary', type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parent.parent
if args.version is None:
    import tomllib
    args.version = tomllib.loads((root / 'Cargo.toml').read_text())['workspace']['package']['version']
binary = args.binary or root / 'target/debug/entanglo-linux'
dist = root / 'dist'
dist.mkdir(exist_ok=True)
shutil.copy2(root / 'packaging/INSTALL.txt', dist / 'INSTALL.txt')
with tempfile.TemporaryDirectory(prefix='entanglo-package-') as temporary:
    stage = Path(temporary)
    def write(name, content, mode=0o644):
        target = stage / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)
        target.chmod(mode)
    def copy(source, name, mode=0o644):
        target = stage / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
        target.chmod(mode)
    copy(binary, 'usr/bin/entanglo-linux', 0o755)
    subprocess.run(['strip', '--strip-debug', str(stage / 'usr/bin/entanglo-linux')], check=True)
    write('DEBIAN/control', f'''Package: entanglo-linux
Version: {args.version}
Architecture: amd64
Maintainer: Paolo Asara <paolo.asara86@gmail.com>
Section: net
Priority: optional
Depends: libc6 (>= 2.38), libgcc-s1, libgtk-4-1 (>= 4.10), libadwaita-1-0 (>= 1.4), libglib2.0-0t64, libudev1, udev, kmod, passwd, login, xdg-utils, curl, ca-certificates, avahi-daemon, avahi-utils
Description: Entanglo LAN mouse, keyboard and file reception
 Receive input and files from trusted Entanglo devices on your LAN.
 X11 input reception supports macOS 0.1.58 absolute pointer coordinates.
''')
    write('DEBIAN/postinst', '''#!/bin/sh
set -e
if [ "$1" = configure ]; then
    modprobe uinput
    udevadm control --reload-rules
    udevadm trigger --action=change --sysname-match=uinput
fi
''', 0o755)
    write('DEBIAN/postrm', '''#!/bin/sh
set -e
if command -v udevadm >/dev/null 2>&1; then udevadm control --reload-rules || true; fi
''', 0o755)
    copy(root / 'packaging/60-entanglo-uinput.rules', 'usr/lib/udev/rules.d/60-entanglo-uinput.rules')
    copy(root / 'packaging/entanglo-uinput.conf', 'usr/lib/modules-load.d/entanglo-uinput.conf')
    launcher = (root / 'scripts/start-receiver.sh').read_text().replace('$HOME/.local/bin/entanglo-linux', '/usr/bin/entanglo-linux')
    write('usr/bin/entanglo-receiver', launcher, 0o755)
    write('usr/bin/entanglo-stop', '#!/bin/sh\npkill -TERM -x entanglo-linux || true\n', 0o755)
    write('usr/share/applications/com.paoloasara.Entanglo.desktop', '''[Desktop Entry]
Type=Application
Name=Entanglo
Comment=Receive mouse, keyboard and files from trusted LAN devices
Exec=/usr/bin/entanglo-linux
Icon=com.paoloasara.Entanglo
StartupWMClass=entanglo-linux
Terminal=false
Categories=Network;Utility;
''')
    write('usr/share/applications/entanglo-stop.desktop', '''[Desktop Entry]
Type=Application
Name=Stop Entanglo
Exec=/usr/bin/entanglo-stop
Icon=process-stop
Terminal=false
Categories=Utility;
''')
    for icon in (root / 'crates/entanglo-linux/data/icons/hicolor').glob('*/apps/*.png'):
        relative = icon.relative_to(root / 'crates/entanglo-linux/data/icons')
        copy(icon, str(Path('usr/share/icons') / relative))
    if (dist / 'INSTALL.txt').exists(): copy(dist / 'INSTALL.txt', 'usr/share/doc/entanglo-linux/INSTALL.txt')
    write('usr/share/doc/entanglo-linux/BUILD-INFO', f'Version: {args.version}\nSHA256 of packaged executable: {hashlib.sha256((stage / "usr/bin/entanglo-linux").read_bytes()).hexdigest()}\n')
    package = dist / f'entanglo-linux_{args.version}_amd64.deb'
    subprocess.run(['dpkg-deb', '--root-owner-group', '-Zxz', '--build', str(stage), str(package)], check=True)
installer = dist / 'install-entanglo.sh'
installer.write_text((root / 'packaging/install.sh').read_text().replace('@PACKAGE@', package.name))
installer.chmod(0o755)
(dist / 'SHA256SUMS').write_text(''.join(hashlib.sha256(p.read_bytes()).hexdigest() + '  ' + p.name + '\n' for p in sorted(dist.iterdir()) if p.is_file() and p.name != 'SHA256SUMS'))
archive = root / 'entanglo-linux-imac-debian13-amd64.tar.gz'
with tarfile.open(archive, 'w:gz') as output: output.add(dist, arcname='entanglo-linux-install')
print(f'Package: {package}\nTransferable archive: {archive}')
