
> 📖 🇪🇸 También disponible en español: [README.es.md](README.es.md)

![p2p-video-chat](/p2p-video-chat.png)

![Rust](https://img.shields.io/badge/Rust-000000?logo=rust&logoColor=white)
![Iroh](https://img.shields.io/badge/Iroh-6A1B9A?logo=data:https://www.iroh.computer/)
![CLI](https://img.shields.io/badge/CLI-222222?logo=gnubash&logoColor=white)
![License](https://img.shields.io/badge/License-MIT-brown)


this is a CLI application for peer-to-peer video chat using the Iroh crate and the gossip protocol.

## How to Use

### Download the Binary
1. Go to the [latest release](../../releases/latest)
2. Download the binary file (p2p-video-chat)

### Connecting with Someone

1. **Person A** runs: `./p2p-video-chat open`
2. **Person A** shares the room code that appears
3. **Person B** runs: `./p2p-video-chat join <room-code>`
4. You're connected!

## Requirements

- Camera access (ideal for video chat)
- Internet connection
- Terminal/command line

## Notes
- Maximum 2 people per room
- The connection is peer-to-peer (direct between you and your friend)
- No data goes through external servers once connected
- close the terminal or press Ctrl+C to exit

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file.