/*
 * Copyright (c) Kia Shakiba
 *
 * This source code is licensed under the GNU AGPLv3 license found in the
 * LICENSE file in the root directory of this source tree.
 */

use std::process;

const ASCII_LOGO: &str = r"
      _____
     |  __ \
     | |__) |_ _ _ __   ___ _ __     PaperCache v<VERSION>
     |  ___/ _` | '_ \ / _ \ '__|    PORT: <PORT>
     | |  | (_| | |_) |  __/ |       PID:  <PID>
     |_|   \__,_| .__/ \___|_|
                | |
                |_|

";

pub fn print(version: &str, port: u32) {
	let logo = ASCII_LOGO
		.replace("<VERSION>", version)
		.replace("<PORT>", &port.to_string())
		.replace("<PID>", &process::id().to_string());

	println!("{logo}");
}
