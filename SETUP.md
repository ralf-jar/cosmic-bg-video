# Configuración de Dependencias para Pop!_OS 24

Para ejecutar el proyecto `cosmic-bg-video` en una instalación limpia de Pop!_OS 24 (basada en Ubuntu), necesitarás instalar el entorno de desarrollo de Rust y varias librerías de sistema para Wayland, GStreamer y la interfaz gráfica.

## 1. Instalar el Toolchain de Rust

La forma recomendada de instalar Rust es usando `rustup`. Si aún no lo tienes, abre una terminal y ejecuta el siguiente comando:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Sigue las instrucciones en pantalla. Una vez completado, asegúrate de que `cargo` esté en tu `PATH` reiniciando tu terminal o ejecutando:

```bash
source "$HOME/.cargo/env"
```

## 2. Instalar Dependencias del Sistema

Estas son las librerías necesarias para compilar y ejecutar tanto el núcleo del reproductor de video (`cosmic-bg-core`) como la herramienta de configuración gráfica (`cosmic-bg-config`).

Puedes instalarlas todas con un solo comando:

```bash
sudo apt update
sudo apt install build-essential pkg-config libwayland-dev libxkbcommon-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly gstreamer1.0-libav libfontconfig1-dev libfreetype6-dev libexpat1-dev
```

### Resumen de las Dependencias Instaladas:

*   **`build-essential`**: Proporciona herramientas de compilación básicas como `gcc` y `make`, esenciales para construir muchos paquetes de Rust que tienen componentes C.
*   **`pkg-config`**: Una herramienta que ayuda a los scripts de compilación a encontrar las librerías de sistema y sus cabeceras.
*   **`libwayland-dev`**: Cabeceras de desarrollo para el protocolo Wayland, necesarias para que la aplicación se comunique con el compositor Wayland.
*   **`libxkbcommon-dev`**: Cabeceras de desarrollo para la librería `xkbcommon`, que gestiona la entrada de teclado en Wayland.
*   **`libgstreamer1.0-dev`**: Cabeceras de desarrollo para el framework GStreamer principal.
*   **`libgstreamer-plugins-base1.0-dev`**: Cabeceras de desarrollo para los plugins base de GStreamer.
*   **`gstreamer1.0-plugins-good`**: Un conjunto de plugins de GStreamer de buena calidad, a menudo necesarios para la decodificación de video estándar.
*   **`gstreamer1.0-plugins-bad`**: Plugins de GStreamer que tienen problemas de calidad o licencias más restrictivas, pero que a menudo son necesarios para formatos de video menos comunes.
*   **`gstreamer1.0-plugins-ugly`**: Plugins de GStreamer con códecs de video con licencias más problemáticas, a menudo necesarios para la compatibilidad con formatos propietarios.
*   **`gstreamer1.0-libav`**: Proporciona integración con la librería `libav` (FFmpeg) para GStreamer, lo que permite la decodificación de una amplia gama de formatos de video.
*   **`libfontconfig1-dev`, `libfreetype6-dev`, `libexpat1-dev`**: Dependencias comunes para librerías de renderizado de texto y fuentes, utilizadas por la interfaz gráfica `iced` en `cosmic-bg-config`.

Con estas dependencias instaladas, deberías poder compilar y ejecutar ambos componentes del proyecto.