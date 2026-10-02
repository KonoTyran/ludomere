pkgname=ludomere
pkgver=0.2.4
pkgrel=1
pkgdesc='A native GOG library, download, and game manager for Linux.'
arch=('x86_64')
license=('GPL-3.0-or-later')
# Rust release binaries are already stripped; preserve official helper bytes.
options=('!lto' '!debug' '!strip')
depends=('gtk4>=4.14' 'libadwaita>=1.5' 'gdk-pixbuf2' 'webkitgtk-6.0>=2.50' 'libsecret' 'dbus' 'xz' 'python' 'python-xlib' 'python-urllib3')
optdepends=('gnome-keyring: Secret Service provider for login (or another provider)' 'vulkan-driver: Windows game graphics' 'lib32-vulkan-driver: 32-bit Windows game graphics')
makedepends=('rust>=1.92' 'pkgconf')
source=("$pkgname-$pkgver.tar.gz")
# tools/build-package.py inserts the current source snapshot checksum.
sha256sums=('@SOURCE_SHA256@')

prepare() {
  cd "$pkgname-$pkgver"
  cargo fetch --locked --target "$CARCH-unknown-linux-gnu"
  python3 tools/prepare-helpers.py --destination target/helpers --cache "${LUDOMERE_HELPER_CACHE:-$srcdir/helper-downloads}"
}

build() {
  cd "$pkgname-$pkgver"
  cargo build --frozen --release
}

check() {
  cd "$pkgname-$pkgver"
  bash tools/check.sh
}

package() {
  cd "$pkgname-$pkgver"
  install -Dm755 "${CARGO_TARGET_DIR:-target}/release/ludomere" "$pkgdir/usr/bin/ludomere"
  install -d "$pkgdir/usr/lib/ludomere" "$pkgdir/usr/share/licenses/$pkgname" "$pkgdir/usr/share/doc/$pkgname"
  cp -a target/helpers/umu target/helpers/comet "$pkgdir/usr/lib/ludomere/"
  cp -a target/helpers/licenses/. "$pkgdir/usr/share/licenses/$pkgname/"
  install -Dm644 resources/licenses/VDF-LICENSE.txt "$pkgdir/usr/share/licenses/$pkgname/VDF-LICENSE"
  cp -a target/helpers/sources "$pkgdir/usr/share/doc/$pkgname/"
  install -Dm644 resources/io.github.KonoTyran.Ludomere.desktop \
    "$pkgdir/usr/share/applications/io.github.KonoTyran.Ludomere.desktop"
  install -Dm644 resources/io.github.KonoTyran.Ludomere.metainfo.xml \
    "$pkgdir/usr/share/metainfo/io.github.KonoTyran.Ludomere.metainfo.xml"
  install -Dm644 resources/icons/io.github.KonoTyran.Ludomere.svg \
    "$pkgdir/usr/share/icons/hicolor/scalable/apps/io.github.KonoTyran.Ludomere.svg"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 THIRD_PARTY_NOTICES.md \
    "$pkgdir/usr/share/licenses/$pkgname/THIRD_PARTY_NOTICES.md"
  install -Dm644 resources/icons/platform/LICENSE.fontawesome.txt \
    "$pkgdir/usr/share/licenses/$pkgname/LICENSE.fontawesome.txt"
  install -Dm644 resources/icons/platform/LICENSE.CC-BY-4.0.txt \
    "$pkgdir/usr/share/licenses/$pkgname/LICENSE.CC-BY-4.0.txt"
}
