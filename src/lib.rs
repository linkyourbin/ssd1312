#![no_std]
use embassy_stm32::i2c::Error;
use embassy_time::Delay;
use embedded_hal::i2c::I2c as I2cTrait;
use embedded_hal::delay::DelayNs;

// embedded-graphics相关导入
use embedded_graphics::{
    draw_target::DrawTarget, geometry::{Dimensions, OriginDimensions, Size}, mono_font::{ascii::{FONT_6X10, FONT_8X13}, MonoTextStyle}, pixelcolor::BinaryColor, prelude::*, primitives::{Line, PrimitiveStyle, Rectangle, StyledDrawable}, text::{Baseline, Text, TextStyle}
};

/// SSD1312 I2C从地址（SA0=0时，对应D/C#接VSS）
const SSD1312_I2C_ADDR: u8 = 0x3C;
/// SSD1312显示屏尺寸
const SCREEN_WIDTH: u8 = 128;
const SCREEN_HEIGHT: u8 = 64;
const PAGE_COUNT: u8 = 8;

/// SSD1312驱动结构体
pub struct Ssd1312<I2C> {
    i2c: I2C,
    buffer: [u8; 1024], // 128x64/8 = 1024字节显存缓冲区
}

impl<I2C: I2cTrait<Error = Error>> Ssd1312<I2C> {
    /// 创建新的SSD1312驱动实例
    pub fn new(i2c: I2C) -> Self {
        Self { 
            i2c,
            buffer: [0u8; 1024],
        }
    }

    /// 发送命令到SSD1312（手册6.1.5节 I2C写入模式）
    fn send_command(&mut self, cmd: u8) -> Result<(), Error> {
        let data = [0x00, cmd]; // 控制字节(Co=0, D/C#=0) + 命令
        self.i2c.write(SSD1312_I2C_ADDR, &data)
    }

    /// 发送多个命令
    fn send_commands(&mut self, cmds: &[u8]) -> Result<(), Error> {
        for &cmd in cmds {
            self.send_command(cmd)?;
        }
        Ok(())
    }

    /// 发送数据到SSD1312（手册6.1.5节）
    fn send_data(&mut self, data: &[u8]) -> Result<(), Error> {
        if data.is_empty() {
            return Ok(());
        }
        
        let mut buf = [0u8; 129]; // 1字节控制+128字节数据
        buf[0] = 0x40; // 控制字节(Co=0, D/C#=1)
        let len = data.len().min(128);
        buf[1..1+len].copy_from_slice(&data[..len]);
        self.i2c.write(SSD1312_I2C_ADDR, &buf[..1+len])
    }

    /// 设置页地址（手册2.1.14节）
    fn set_page(&mut self, page: u8) -> Result<(), Error> {
        if page < 8 {
            self.send_command(0xB0 | page)
        } else {
            Ok(())
        }
    }

    /// 设置列地址（手册2.1.1和2.1.2节）
    fn set_column(&mut self, col: u8) -> Result<(), Error> {
        if col < 128 {
            self.send_command(col & 0x0F)?;        // 低4位
            self.send_command(0x10 | (col >> 4))   // 高4位
        } else {
            Ok(())
        }
    }

    /// 初始化SSD1312（手册6.9节和Table 1-1）
    pub fn init(&mut self, delay: &mut Delay) -> Result<(), Error> {
        // 按照手册6.9.2节的电荷泵上电序列
        
        // 1. 等待VDD稳定（至少20ms）
        delay.delay_ms(20u32);
        
        // 2. 显示关闭
        self.send_command(0xAE)?;
        
        // 3. 设置显示时钟分频比/振荡器频率（手册2.1.17节）
        self.send_commands(&[0xD5, 0x80])?; // 分频比=1，默认频率
        
        // 4. 设置复用比（手册2.1.11节）
        self.send_commands(&[0xA8, 0x3F])?; // 64MUX (63+1)
        
        // 5. 设置显示偏移（手册2.1.16节）
        self.send_commands(&[0xD3, 0x00])?; // 无偏移
        
        // 6. 设置显示起始行（手册2.1.6节）
        self.send_command(0x40)?; // 起始行=0
        
        // 7. 启用内部电荷泵（手册2.1.22节）
        self.send_commands(&[0x8D, 0x12])?; // 启用电荷泵，7.5V模式
        
        // 20 02 A0 C8
        // 20 02 A1 C0
        // 20 09 A1 C8
        // 20 09 A0 C0

        // 8. 内存寻址模式（手册2.1.3节）
        self.send_commands(&[0x20, 0x02])?; // 页寻址模式
        
        // 9. 段重映射（手册2.1.8节）
        self.send_command(0xA0)?; // 列地址0映射到SEG0
        
        // 10. COM输出扫描方向（手册2.1.15节）
        self.send_command(0xC8)?; // 垂直翻转
        
        // 11. SEG引脚硬件配置（手册2.1.19节）
        self.send_commands(&[0xDA, 0x12])?; // 交替SEG引脚配置
        
        // 12. 设置对比度（手册2.1.7节）
        self.send_commands(&[0x81, 0x7F])?; // 默认对比度
        
        // 13. 设置预充电周期（手册2.1.18节）
        self.send_commands(&[0xD9, 0x22])?; // Phase1=2, Phase2=2
        
        // 14. 设置VCOMH电压（手册2.1.20节）
        self.send_commands(&[0xDB, 0x20])?; // ~0.77 x VCC
        
        // 15. 恢复RAM内容显示（手册2.1.9节）
        self.send_command(0xA4)?;
        
        // 16. 正常显示模式（手册2.1.10节）
        self.send_command(0xA6)?;
        
        // 17. 开启显示（手册2.1.13节）
        self.send_command(0xAF)?;
        
        // 18. 等待显示稳定（手册6.9.2节，至少100ms）
        delay.delay_ms(100u32);
        
        Ok(())
    }

    /// 清除屏幕缓冲区
    pub fn clear_buffer(&mut self) {
        self.buffer.fill(0);
    }

    /// 在缓冲区中设置像素
    pub fn set_pixel(&mut self, x: u8, y: u8, on: bool) {
        if x >= SCREEN_WIDTH || y >= SCREEN_HEIGHT {
            return;
        }
        
        let page = (y / 8) as usize;
        let bit_pos = y % 8;
        let index = page * SCREEN_WIDTH as usize + x as usize;
        
        if index < self.buffer.len() {
            if on {
                self.buffer[index] |= 1 << bit_pos;
            } else {
                self.buffer[index] &= !(1 << bit_pos);
            }
        }
    }

    /// 将缓冲区内容写入显示屏
    pub fn display(&mut self) -> Result<(), Error> {
        for page in 0..PAGE_COUNT {
            self.set_page(page)?;
            self.set_column(0)?;
            
            let start_idx = page as usize * SCREEN_WIDTH as usize;
            let end_idx = start_idx + SCREEN_WIDTH as usize;
            
            if end_idx <= self.buffer.len() {
                let mut page_data = [0u8; 128];
                page_data.copy_from_slice(&self.buffer[start_idx..end_idx]);
                self.send_data(&page_data)?;
            }
        }
        Ok(())
    }

    /// 清除整个显示屏（直接写入硬件）
    pub fn clear(&mut self) -> Result<(), Error> {
        self.clear_buffer();
        self.display()
    }

    /// 绘制单个像素（立即显示）
    pub fn draw_pixel(&mut self, x: u8, y: u8) -> Result<(), Error> {
        self.set_pixel(x, y, true);
        
        // 只更新对应的页
        if x < SCREEN_WIDTH && y < SCREEN_HEIGHT {
            let page = y / 8;
            self.set_page(page)?;
            self.set_column(x)?;
            
            let index = page as usize * SCREEN_WIDTH as usize + x as usize;
            if index < self.buffer.len() {
                let pixel_data = self.buffer[index];
                self.send_data(&[pixel_data])?;
            }
        }
        Ok(())
    }

    /// 绘制水平线
    pub fn draw_horizontal_line(&mut self, x: u8, y: u8, width: u8) -> Result<(), Error> {
        for i in 0..width {
            if x + i < SCREEN_WIDTH {
                self.set_pixel(x + i, y, true);
            }
        }
        self.display()
    }

    /// 绘制垂直线
    pub fn draw_vertical_line(&mut self, x: u8, y: u8, height: u8) -> Result<(), Error> {
        for i in 0..height {
            if y + i < SCREEN_HEIGHT {
                self.set_pixel(x, y + i, true);
            }
        }
        self.display()
    }

    /// 绘制矩形边框
    pub fn draw_rect(&mut self, x: u8, y: u8, width: u8, height: u8) -> Result<(), Error> {
        // 使用embedded-graphics绘制
        let rect = Rectangle::new(Point::new(x as i32, y as i32), Size::new(width as u32, height as u32));
        let style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
        rect.draw_styled(&style, self).map_err(|_| Error::Overrun)?;
        self.display()
    }

    /// 填充矩形
    pub fn fill_rect(&mut self, x: u8, y: u8, width: u8, height: u8) -> Result<(), Error> {
        // 使用embedded-graphics绘制
        let rect = Rectangle::new(Point::new(x as i32, y as i32), Size::new(width as u32, height as u32));
        let style = PrimitiveStyle::with_fill(BinaryColor::On);
        rect.draw_styled(&style, self).map_err(|_| Error::Overrun)?;
        self.display()
    }

    /// 绘制文本 - 小字体（6x10）
    pub fn draw_text_small(&mut self, text: &str, x: i32, y: i32) -> Result<(), Error> {
        let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
        let text_drawable = Text::with_baseline(text, Point::new(x, y), text_style, Baseline::Top);
        text_drawable.draw(self).map_err(|_| Error::Overrun)?;
        self.display()
    }

    /// 绘制文本 - 中等字体（8x13）
    pub fn draw_text_medium(&mut self, text: &str, x: i32, y: i32) -> Result<(), Error> {
        let text_style = MonoTextStyle::new(&FONT_8X13, BinaryColor::On);
        let text_drawable = Text::with_baseline(text, Point::new(x, y), text_style, Baseline::Top);
        text_drawable.draw(self).map_err(|_| Error::Overrun)?;
        self.display()
    }

    /// 绘制居中文本
    pub fn draw_text_centered(&mut self, text: &str, y: i32, font_width: i32) -> Result<(), Error> {
        let text_width = text.len() as i32 * font_width;
        let x = (SCREEN_WIDTH as i32 - text_width) / 2;
        self.draw_text_small(text, x, y)
    }

    /// 绘制线条
    pub fn draw_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) -> Result<(), Error> {
        let line = Line::new(Point::new(x0, y0), Point::new(x1, y1));
        let style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
        line.draw_styled(&style, self).map_err(|_| Error::Overrun)?;
        self.display()
    }

    /// 设置显示开关（手册2.1.13节）
    pub fn set_display_on(&mut self, on: bool) -> Result<(), Error> {
        if on {
            self.send_command(0xAF) // 显示开
        } else {
            self.send_command(0xAE) // 显示关
        }
    }

    /// 设置对比度（手册2.1.7节）
    pub fn set_contrast(&mut self, contrast: u8) -> Result<(), Error> {
        self.send_commands(&[0x81, contrast])
    }

    /// 设置显示反色（手册2.1.10节）
    pub fn set_invert(&mut self, invert: bool) -> Result<(), Error> {
        if invert {
            self.send_command(0xA7) // 反色显示
        } else {
            self.send_command(0xA6) // 正常显示
        }
    }
}

// 实现embedded-graphics的DrawTarget trait
impl<I2C: I2cTrait<Error = Error>> DrawTarget for Ssd1312<I2C> {
    type Color = BinaryColor;
    type Error = ();

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels.into_iter() {
            if coord.x >= 0 && coord.x < SCREEN_WIDTH as i32 
                && coord.y >= 0 && coord.y < SCREEN_HEIGHT as i32 {
                self.set_pixel(coord.x as u8, coord.y as u8, color.is_on());
            }
        }
        Ok(())
    }
}

// 实现embedded-graphics的OriginDimensions trait
impl<I2C: I2cTrait<Error = Error>> OriginDimensions for Ssd1312<I2C> {
    fn size(&self) -> Size {
        Size::new(SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32)
    }
}

/// 便利函数：创建不同样式的文本样式
pub struct TextStyles;

impl TextStyles {
    /// 小字体样式（6x10）
    pub fn small() -> MonoTextStyle<'static, BinaryColor> {
        MonoTextStyle::new(&FONT_6X10, BinaryColor::On)
    }

    /// 中等字体样式（8x13）
    pub fn medium() -> MonoTextStyle<'static, BinaryColor> {
        MonoTextStyle::new(&FONT_8X13, BinaryColor::On)
    }

    /// 反色小字体样式
    pub fn small_inverted() -> MonoTextStyle<'static, BinaryColor> {
        MonoTextStyle::new(&FONT_6X10, BinaryColor::Off)
    }
}