use super::Theme;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Palette {
  pub background: [u8; 3],
  pub foreground: [u8; 3],
  pub surface: [u8; 3],
  pub selection: [u8; 3],
  pub muted: [u8; 3],
  pub blue: [u8; 3],
  pub magenta: [u8; 3],
  pub yellow: [u8; 3],
  pub cyan: [u8; 3],
  pub red: [u8; 3],
}

const fn rgb(hex: u32) -> [u8; 3] {
  [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}

impl Theme {
  pub const fn palette(self) -> Palette {
    match self {
      Self::Default => Palette {
        background: rgb(0x18_1b_24),
        foreground: rgb(0xd8_de_e9),
        surface: rgb(0x4c_56_6a),
        selection: rgb(0x43_4c_5e),
        muted: rgb(0x4c_56_6a),
        blue: rgb(0x81_a1_c1),
        magenta: rgb(0xb4_8e_ad),
        yellow: rgb(0xeb_cb_8b),
        cyan: rgb(0x88_c0_d0),
        red: rgb(0xbf_61_6a),
      },
      Self::TokyonightNight => Palette {
        background: rgb(0x1a_1b_26),
        foreground: rgb(0xc0_ca_f5),
        surface: rgb(0x29_2e_42),
        selection: rgb(0x39_4b_70),
        muted: rgb(0x73_7a_a2),
        blue: rgb(0x7a_a2_f7),
        magenta: rgb(0xbb_9a_f7),
        yellow: rgb(0xe0_af_68),
        cyan: rgb(0x7d_cf_ff),
        red: rgb(0xf7_76_8e),
      },
      Self::Dracula => Palette {
        background: rgb(0x28_2a_36),
        foreground: rgb(0xf8_f8_f2),
        surface: rgb(0x44_47_5a),
        selection: rgb(0x44_47_5a),
        muted: rgb(0x62_72_a4),
        blue: rgb(0xbd_93_f9),
        magenta: rgb(0xff_79_c6),
        yellow: rgb(0xf1_fa_8c),
        cyan: rgb(0x8b_e9_fd),
        red: rgb(0xff_55_55),
      },
      Self::GithubLight => Palette {
        background: rgb(0xff_ff_ff),
        foreground: rgb(0x1f_23_28),
        surface: rgb(0xf6_f8_fa),
        selection: rgb(0xdd_f4_ff),
        muted: rgb(0x59_63_6e),
        blue: rgb(0x09_69_da),
        magenta: rgb(0x82_50_df),
        yellow: rgb(0x9a_67_00),
        cyan: rgb(0x05_50_ae),
        red: rgb(0xcf_22_2e),
      },
      Self::GithubDark => Palette {
        background: rgb(0x0d_11_17),
        foreground: rgb(0xf0_f6_fc),
        surface: rgb(0x15_1b_23),
        selection: rgb(0x21_28_30),
        muted: rgb(0x91_98_a1),
        blue: rgb(0x58_a6_ff),
        magenta: rgb(0xbe_8f_ff),
        yellow: rgb(0xd2_99_22),
        cyan: rgb(0x79_c0_ff),
        red: rgb(0xff_7b_72),
      },
      Self::Nord => Palette {
        background: rgb(0x2e_34_40),
        foreground: rgb(0xd8_de_e9),
        surface: rgb(0x3b_42_52),
        selection: rgb(0x43_4c_5e),
        muted: rgb(0x4c_56_6a),
        blue: rgb(0x81_a1_c1),
        magenta: rgb(0xb4_8e_ad),
        yellow: rgb(0xeb_cb_8b),
        cyan: rgb(0x88_c0_d0),
        red: rgb(0xbf_61_6a),
      },
    }
  }
}
