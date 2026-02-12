use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Cursor, Write};

#[derive(Debug, Clone)]
pub enum GamePacket {
    Binary {
        magic: u16,
        length: u32,
        command: u16,
        qq_num: u64,
        data: Vec<u8>,
    },
    Text(String),
}

#[derive(Debug, Clone)]
pub enum PacketAction {
    Forward,
    Modified(GamePacket),
    Drop,
    Inject(GamePacket),
}

pub trait PacketHandler: Send + Sync {
    fn handle_outbound(&self, packet: &GamePacket) -> PacketAction;
    fn handle_inbound(&self, packet: &GamePacket) -> PacketAction;
}

impl GamePacket {
    pub fn parse(data: &[u8]) -> Result<Self, crate::wpe::WpeError> {
        if data.len() < 2 {
            return Err(crate::wpe::WpeError::PacketParse("Packet too short".to_string()));
        }

        let mut cursor = Cursor::new(data);
        let magic = cursor.read_u16::<LittleEndian>()
            .map_err(|e| crate::wpe::WpeError::PacketParse(format!("Failed to read magic: {}", e)))?;

        if magic == 0x9527 {
            if data.len() < 16 {
                return Err(crate::wpe::WpeError::PacketParse("Binary packet too short".to_string()));
            }

            let length = cursor.read_u32::<LittleEndian>()
                .map_err(|e| crate::wpe::WpeError::PacketParse(format!("Failed to read length: {}", e)))?;
            let command = cursor.read_u16::<LittleEndian>()
                .map_err(|e| crate::wpe::WpeError::PacketParse(format!("Failed to read command: {}", e)))?;
            let qq_num = cursor.read_u64::<LittleEndian>()
                .map_err(|e| crate::wpe::WpeError::PacketParse(format!("Failed to read qq_num: {}", e)))?;

            let remaining = &data[16..];
            Ok(GamePacket::Binary {
                magic,
                length,
                command,
                qq_num,
                data: remaining.to_vec(),
            })
        } else {
            let text = String::from_utf8_lossy(data).to_string();
            Ok(GamePacket::Text(text))
        }
    }

    pub fn build(&self) -> Result<Vec<u8>, crate::wpe::WpeError> {
        match self {
            GamePacket::Binary { magic, length, command, qq_num, data } => {
                let mut buffer = Vec::new();
                buffer.write_u16::<LittleEndian>(*magic)
                    .map_err(|e| crate::wpe::WpeError::PacketBuild(format!("Failed to write magic: {}", e)))?;
                buffer.write_u32::<LittleEndian>(*length)
                    .map_err(|e| crate::wpe::WpeError::PacketBuild(format!("Failed to write length: {}", e)))?;
                buffer.write_u16::<LittleEndian>(*command)
                    .map_err(|e| crate::wpe::WpeError::PacketBuild(format!("Failed to write command: {}", e)))?;
                buffer.write_u64::<LittleEndian>(*qq_num)
                    .map_err(|e| crate::wpe::WpeError::PacketBuild(format!("Failed to write qq_num: {}", e)))?;
                buffer.write_all(data)
                    .map_err(|e| crate::wpe::WpeError::PacketBuild(format!("Failed to write data: {}", e)))?;
                Ok(buffer)
            }
            GamePacket::Text(text) => {
                Ok(text.as_bytes().to_vec())
            }
        }
    }

    pub fn build_map_jump(qq_num: u64, map_no: u16) -> Self {
        let mut data = Vec::new();
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00]);
        data.push((map_no & 0xFF) as u8);
        data.push(((map_no >> 8) & 0xFF) as u8);

        GamePacket::Binary {
            magic: 0x9527,
            length: 0x0B,
            command: 0x0003,
            qq_num,
            data,
        }
    }

    pub fn build_pet_storage(qq_num: u64, spirit_pos: u8) -> Self {
        let mut data = Vec::new();
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x0]);
        data.push(spirit_pos);

        GamePacket::Binary {
            magic: 0x9527,
            length: 0x0B,
            command: 0x0014,
            qq_num,
            data,
        }
    }

    pub fn build_pet_escape() -> Self {
        GamePacket::Text("System_宠物逃跑".to_string())
    }

    pub fn build_home_training(qq_num: u64, spirit_pos: u8) -> Self {
        let mut data = Vec::new();
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x0]);
        data.push(spirit_pos);

        GamePacket::Binary {
            magic: 0x9527,
            length: 0x0B,
            command: 0x0052,
            qq_num,
            data,
        }
    }
}
