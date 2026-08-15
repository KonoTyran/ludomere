pkgname=ludomere
pkgver=0.1.0
pkgrel=1
pkgdesc='A native GOG library, download, and game manager for Linux.'
arch=('x86_64')
license=('GPL-3.0-or-later')
options=('!lto' '!debug')
depends=('gtk4' 'libadwaita' 'gdk-pixbuf2' 'webkit2gtk-4.1' 'libsecret')
optdepends=('umu-launcher: install and run Windows games')
makedepends=('rust')
source=("$pkgname-$pkgver.tar.gz")
sha256sums=('SKIP')

prepare() {
  cd "$pkgname-$pkgver"
  cargo fetch --locked --target "$CARCH-unknown-linux-gnu"
}

build() {
  cd "$pkgname-$pkgver"
  CARGO_TARGET_DIR=target cargo build --frozen --release
}

check() {
  cd "$pkgname-$pkgver"
  CARGO_TARGET_DIR=target cargo test --frozen
}

package() {
  cd "$pkgname-$pkgver"
  install -Dm755 target/release/ludomere "$pkgdir/usr/bin/ludomere"
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
