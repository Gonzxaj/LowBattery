# lowbattery

Monitor de batería de periféricos inalámbricos para Windows 10/11 y Linux, escrito en Rust con UI nativa [CreamUI](https://github.com/sammwyy/creamui). Corre en un solo proceso, no instala servicios, no hace telemetría y no necesita permisos de administrador ni el software del fabricante.

- **Bluetooth estándar:** lee el mismo nivel que muestra Ajustes de Windows.
- **Logitech HID++ 2.0:** receptores Unifying, Bolt y Lightspeed, y conexión directa.
- **SteelSeries:** Aerox, Prime, Rival 3 Wireless, Arctis 7/7P/7+/Nova 5/Nova 7/Nova Pro Wireless.
- **Razer:** mouse y teclados inalámbricos y sus dongles, sin tabla de modelos.
- **Teclados con canal de batería `FF1C:0092`, de cualquier marca:** firmware Telink/Evision que venden FEKER, VGN, Rainy y otras. Se reconocen por ese canal, no por marca, tanto por 2.4G como por cable, con nivel y carga. Hay una sola tarjeta por teclado, use el cable o el receptor.
- **Receptores 2.4G genéricos sin batería** (por ejemplo YICHIP): aparecen como conectados, con una nota de que no informan la batería, y no se muestran en el widget.
- **Cualquier marca:** dispositivos HID que declaran el uso estándar "Battery Strength".
- **Tray:** el ícono muestra el nivel más bajo y el tooltip lista todos los dispositivos. Avisa con un toast de Windows cuando un dispositivo llega a batería baja.
- **Widget flotante:** sin bordes, con opacidad configurable, opción de siempre visible y selección de qué dispositivos mostrar.

## Compilar (Ubuntu, solo Docker)

```sh
./build.sh test        # tests unitarios (protocolos con dispositivos simulados)
./build.sh check       # clippy para Windows
./build.sh release     # dist/lowbattery.exe
./build.sh installer   # dist/lowbattery-0.1.0.0.msix (Store) y -sideload.msix (firmado para pruebas)
```

Smoke test de la UI con Wine (`--demo` usa dispositivos simulados):

```sh
docker build -t lowbattery-wine -f docker/Dockerfile.wine docker/
docker run --rm -u "$(id -u):$(id -g)" -v "$PWD":/src lowbattery-wine /src/docker/smoke.sh
# capturas en dist/smoke-*.png
```

## MSIX y Microsoft Store

`./build.sh installer` arma el paquete con `makemsix`, la herramienta oficial de Microsoft que también funciona en Linux. La primera vez compila la imagen `docker/Dockerfile.msix`. Genera tres archivos:

- **`dist/lowbattery-<ver>.msix`:** sin firmar; es el que se sube a Partner Center, porque la Store lo firma.
- **`dist/lowbattery-<ver>-sideload.msix`:** firmado con un certificado de desarrollo propio, para instalarlo en tu PC.
- **`dist/lowbattery-dev.cer`:** ese certificado.

**Para la Store:**
1. Reservá el nombre en Partner Center.
2. Copiá `installer/msix.env.example` a `installer/msix.env` y pegá `Name`, `Publisher` y `PublisherDisplayName` desde *Product identity*.
3. Volvé a correr `./build.sh installer` y subí el `.msix` sin firmar.

La app declara `runFullTrust`, que es necesario para acceder a HID; Microsoft lo revisa en la certificación.

**Para instalarlo en tu PC (sideload):**
1. Confiá en el certificado una sola vez, desde PowerShell como administrador:
   `Import-Certificate -FilePath lowbattery-dev.cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople`
2. Abrí el `-sideload.msix` con doble clic, o instalalo con `Add-AppxPackage`.

Diferencias cuando la app corre empaquetada:
- **Config y log:** se guardan en `%LOCALAPPDATA%\Packages\<paquete>\LocalState\`.
- **Iniciar con Windows:** usa el `StartupTask` del paquete en lugar de la clave `Run`, y se puede desactivar desde Administrador de tareas > Inicio.
- **Avisos:** salen con la identidad del paquete.

## Uso

- `lowbattery.exe`: abre la ventana principal, el widget y el ícono en la bandeja. Cerrar la ventana la oculta en la bandeja.
- `--minimized`: arranca solo con la bandeja. El inicio con Windows usa este modo.
- `--demo`: usa dispositivos simulados, sin hardware.
- `--debug`: registro detallado.
- Configuración: `%APPDATA%\LowBattery\config.json` (empaquetada: ver arriba).

## Linux

```sh
./build.sh linux   # dist/lowbattery_<ver>_amd64.deb y dist/lowbattery-<ver>-linux-amd64.tar.gz
```

- **`.deb` (Ubuntu, Debian, Mint, Pop!_OS):** `sudo apt install ./dist/lowbattery_*_amd64.deb`. Instala también la regla udev.
- **`.tar.gz` (cualquier distro):** descomprimir y correr `sudo ./install.sh`, que instala en `/usr/local`. Para desinstalar: `sudo ./install.sh --uninstall`.
- **Requisitos:** glibc 2.35 o superior (Ubuntu 22.04+, Debian 12+, Fedora 36+) y X11 o Wayland.

Qué cambia respecto de Windows:
- **Logitech, Bluetooth y HID estándar:** se leen de `/sys/class/power_supply`, que publica el kernel (drivers `hid-logitech-hidpp` y `hid-input`). No necesita permisos.
- **SteelSeries, Razer y teclados `FF1C`:** se leen por `/dev/hidraw*`, que es solo de root hasta instalar `installer/linux/70-lowbattery.rules`. Los paquetes la instalan. Si falta, el log muestra `no permission to open /dev/hidrawN`.
- **Tray:** usa StatusNotifierItem. Anda en KDE, XFCE, Cinnamon y en el GNOME de Ubuntu. En GNOME sin modificar hace falta la extensión *AppIndicator*; sin tray, la ventana principal queda visible.
- **Wayland:** el protocolo no permite que una app elija la posición de su ventana ni la ponga siempre visible, así que esas opciones del widget solo funcionan en X11.
- **Config y log:** `~/.config/LowBattery/`.

### Snap Store y Flathub

- **Snap:** `snap/snapcraft.yaml`. Se compila con `snapcraft` y se publica con `snapcraft register lowbattery` y `snapcraft upload --release=stable`. El confinamiento strict no da acceso a hidraw, así que en el snap solo funcionan Logitech, Bluetooth y HID estándar.
- **Flathub:** `installer/flatpak/io.github.gonzxaj.LowBattery.yml`. Primero correr `installer/flatpak/gen-sources.sh` (genera `cargo-sources.json`) y después `flatpak-builder --user --install build-flatpak installer/flatpak/io.github.gonzxaj.LowBattery.yml`. Para publicar se abre un PR en `flathub/flathub` con el manifest (con source `git` y tag) y `cargo-sources.json`. SteelSeries y Razer igual necesitan la regla udev instalada en el sistema.
- **Metadatos compartidos:** `installer/linux/` tiene el `.desktop`, el metainfo de AppStream (Flathub exige al menos una captura: `assets/screenshots/devices.png`), el ícono y la regla udev.

## Registro y periféricos que no aparecen

La app escribe `%APPDATA%\LowBattery\lowbattery.log`, que se abre desde el tray o con el botón "Ver registro". Contiene:

- **Al iniciar:** un diagnóstico con las interfaces HID de los fabricantes soportados y los nodos Bluetooth con batería.
- **Cambios de estado:** cada dispositivo nuevo y cada cambio de nivel, carga o conexión.
- **Periféricos no soportados:** una línea `UNSUPPORTED peripheral vid_xxxx&pid_xxxx "Producto" ...` por cada periférico detectado que no tiene protocolo de batería todavía, con lo necesario para agregarlo. Si te falta un dispositivo, enviá ese archivo.

Con `--debug` también registra cada ciclo de lectura.

La arquitectura, las APIs, la compatibilidad, las limitaciones y el plan de pruebas con hardware real están en [docs/RESEARCH.md](docs/RESEARCH.md).

## Licencia

Copyright (c) 2026 Gonzalo Carranza (Gonzxaj). Publicado bajo la [licencia MIT](LICENSE).

Incluye una copia modificada de `creamui-render` (Apache-2.0) y otras dependencias de código abierto; ver [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

