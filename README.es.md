![p2p-video-chat](/p2p-video-chat.png)

![Rust](https://img.shields.io/badge/Rust-000000?logo=rust&logoColor=white)
![Iroh](https://img.shields.io/badge/Iroh-6A1B9A?logo=data:https://www.iroh.computer/)
![CLI](https://img.shields.io/badge/CLI-222222?logo=gnubash&logoColor=white)
![License](https://img.shields.io/badge/License-MIT-brown)

Esta es una aplicación CLI para videochat peer-to-peer usando el crate Iroh y el protocolo gossip.

## Cómo Usar

### Descargar el Binario
1. Ve a la [última versión](../../releases/latest)
2. Descarga el archivo binario (p2p-video-chat)

### Conectarse con Alguien

#### En macOS/Linux:
1. **Persona A** ejecuta: `./p2p-video-chat open`
2. **Persona A** comparte el código de sala que aparece
3. **Persona B** ejecuta: `./p2p-video-chat join <código-de-sala>`
4. ¡Ya están conectados!

#### En Windows:
1. **Persona A** ejecuta: `./p2p-video-chat.exe open`
2. **Persona A** comparte el código de sala que aparece
3. **Persona B** ejecuta: `./p2p-video-chat.exe join <código-de-sala>`
4. ¡Ya están conectados!

## Requisitos

- Acceso a cámara (ideal para videochat)
- Conexión a internet
- Terminal/línea de comandos

## Notas
- Máximo 2 personas por sala
- La conexión es peer-to-peer (directa entre tú y tu amigo)
- Ningún dato pasa por servidores externos una vez conectados
- Cierra la terminal o presiona Ctrl+C para salir

## Licencia

Este proyecto está licenciado bajo la Licencia MIT - consulta el archivo [LICENSE](LICENSE).
