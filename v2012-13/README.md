# Minecraft Launcher 2012 (v13)

El launcher original de Minecraft de 2012, versión 13, adaptado para hoy: funciona de nuevo y abre
las versiones nuevas de Minecraft Java, pero sigue siendo el mismo launcher. No se modifica ni un
byte de su `Minecraft.exe`: se ve y se usa como en 2012.

## Qué hace

Un solo `Minecraft.exe` que lleva dentro el launcher original, un agente Java y un servidor local:

- **El launcher de siempre.** Se abre la ventana original, con su login y sus noticias. Los
  servidores de 2012 ya no existen, así que el agente redirige al servidor local las direcciones
  que el launcher pide (`login.minecraft.net`, `s3.amazonaws.com`, `mcupdate.tumblr.com`...).
- **Versiones nuevas.** El juego se descarga de los servidores actuales de Mojang. Hasta la 1.5.2
  (y las alphas y betas) corre dentro de la ventana del launcher, como entonces. Desde la 1.6, y en
  las más antiguas, el launcher muestra su pantalla "Updating Minecraft" y el juego se abre aparte,
  como lo hace el launcher actual de Mojang.
- **Noticias.** El panel de noticias muestra la web de `assets/`: `index.html` primero y, al bajar,
  los posts de `2.html`, `3.html`... con scroll infinito.
- **Perfiles.** El botón *Options* abre un editor de perfiles al estilo del launcher de 2013: nombre,
  usuario, versión, resolución, Java y argumentos JVM de cada perfil, y los del launcher.
- **Solo queda el juego.** Al iniciar el juego se cierran el launcher y este programa. Las versiones
  hasta la 1.5.2 se juegan en la ventana del launcher, así que esa ventana sigue abierta.
- **Sin tocar tu Minecraft.** Todo vive en `%APPDATA%\mc-classic\2012-13`, nunca en
  `%APPDATA%\.minecraft`. Cada perfil tiene su carpeta en `instances\`, con sus propios mundos y
  opciones.

## Qué Java usa

No hay que instalar Java: se descarga solo la primera vez, de los servidores de Mojang.

| Para | Java |
|---|---|
| El launcher | **Java 8** (8u51, `jre-legacy` de Mojang). Es el más nuevo que admite el launcher de 2012, que ejecuta el juego como applet. |
| El juego | El que pide cada versión: Java 8 hasta la 1.16, y Java 16, 17, 21 o 25 en las siguientes. Un perfil puede usar otro (*Executable*). |

## Perfiles

*Options* → editor de perfiles (se guardan en `%APPDATA%\mc-classic\2012-13\profiles.json`):

| Sección | Opción |
|---|---|
| Profile Info | Perfil (*New*, *Delete*), nombre, usuario (rellena el login) y resolución de la ventana del juego. |
| Version Selection | *Use version*: por defecto la última versión publicada (*Latest release*). Las casillas muestran también snapshots, Beta, Alpha y versiones con mods (Fabric). |
| Java Settings | *Executable* (un Java propio), *JVM Arguments* del juego (memoria, como `-Xmx4G`) y *Launcher JVM Arguments*, que se aplican al volver a abrir el launcher. |

Sin marcar, cada opción usa lo de siempre. Las versiones de Fabric traen el cargador de mods
Fabric: los mods van en `instances\<perfil>\.minecraft\mods`.

El login está en modo pruebas: acepta cualquier usuario válido (2 a 16 letras, cifras o `_`) con
cualquier contraseña. El registro queda en `%APPDATA%\minecraft-launcher-classic\server\server.log`.

## Compilar

Hace falta:

- Rust.
- Un JDK que acepte `--release 8` (`JAVA_HOME` o `javac` en el PATH).
- El `Minecraft.exe` original de 2012 (versión 13) en esta carpeta, o en la variable
  `MINECRAFT_EXE`. Es un archivo de Mojang y no está en el repositorio.

```
cd v2012-13
cargo run
```

Para repartirlo, `cargo build --release` deja `target\release\Minecraft.exe` (unos 3,5 MB):
lleva dentro el launcher original, el agente, la web de noticias, y un servidor y un cliente HTTPS.

## Estructura

| Carpeta | Contenido |
|---|---|
| `src/` | El exe en Rust: servidor local, descargas de Mojang y arranque del juego. |
| `agent/` | El agente Java que se carga en el launcher. |
| `bridge/` | El applet que abre las versiones que el launcher no puede ejecutar dentro. |
| `assets/` | La web de noticias y sus imágenes, que van dentro del exe. |
