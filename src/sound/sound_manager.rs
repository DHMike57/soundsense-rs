use super::*;
use regex::Regex;

pub struct SoundManager {
	sounds: Vec<SoundEntry>,
	device: Device,
	channels: HashMap<String, SoundChannel>,
	total_volume: f32,
	concurency: usize,
	ui_sender: glib::Sender<UIMessage>,
	rng: ThreadRng,
}

impl SoundManager {
	pub fn new(sound_dir: &Path, ui_sender: glib::Sender<UIMessage>) -> Self {
		use xml::reader::{EventReader, XmlEvent};

		let mut sounds = Vec::new();
		let device = default_output_device().unwrap();
		let mut channels = HashMap::new();

		channels.insert("misc".to_string(), SoundChannel::new(&device));

		fn visit_dir(dir: &Path, func: &mut FnMut(&Path)) {
			for entry in fs::read_dir(dir).unwrap() {
				let entry = entry.unwrap();
				let path = entry.path();
				if path.is_dir() {
					visit_dir(&path, func);
				} else if path.is_file() && path.extension().unwrap() == "xml" {
					func(&path);
				}
			}
		}

		let mut func = |file_path: &Path| {
			let file = fs::File::open(file_path).unwrap();
			let file = io::BufReader::new(file);
			let parser = EventReader::new(file);

			let mut current_sound : Option<SoundEntry> = None;

			for e in parser {
				match e.unwrap() {
					XmlEvent::StartElement{name, attributes, ..} => {
						if name.local_name == "sound" {

							let mut pattern: Option<Regex> = None;
							let mut channel: Option<String> = None;
							let mut loop_attr: Option<bool> = None;
							let mut concurency: Option<usize> = None;
							let mut timeout: Option<usize> = None;
							let mut probability: Option<usize> = None;
							let mut delay: Option<usize> = None;
							let mut halt_on_match: bool = false;
							let mut random_balance: bool = false;
							let files = Vec::new();

							for attr in attributes {
								let attribute_name = attr.name.local_name.as_str();
								match attribute_name {
									"logPattern" => {
										lazy_static! {
											static ref FAULTY_ESCAPE: Regex = Regex::new(
												r"(?:\\)([^\.\+\*\?\(\)\|\[\]\{\}\^\$])"
											).unwrap();

											static ref EMPTY_EXPR: Regex = Regex::new(
												r"(\|\(\)\))"
											).unwrap();
										}
										let mut processed = attr.value;
										processed = FAULTY_ESCAPE.replace_all(&processed, "$1").into();
										processed = EMPTY_EXPR.replace_all(&processed, ")?").into();
										pattern = Some(Regex::new(&processed).unwrap());
									},
									"channel" => {
										channels.entry(attr.value.clone())
											.or_insert_with(|| SoundChannel::new(&device));
										channel.replace(attr.value.clone());
									},
									"loop" => if attr.value == "start" {
										loop_attr.replace(true);
									}
									else {
										loop_attr.replace(false);
									},
									"concurency" => {
										concurency.replace( attr.value.parse().unwrap() );
									},
									"timeout" => {
										timeout.replace( attr.value.parse().unwrap() );
									},
									// Probability was mispelled...
									"propability" => {
										probability.replace( attr.value.parse().unwrap() );
									},
									"delay" => {
										delay.replace( attr.value.parse().unwrap() );
									},
									"haltOnMatch" => if attr.value == "true" {
										halt_on_match = true;
									},
									"randomBalance" => if attr.value == "true" {
										random_balance = true;
									}
									"ansiFormat" => (),
									"ansiPattern" => (),
									"playbackThreshhold" => (),
									_ => println!("Unknown sound value: {}", attribute_name)
								}
							}

							current_sound = Some(
								SoundEntry{
									pattern: pattern.take().unwrap(),
									channel,
									loop_attr,
									concurency,
									timeout,
									probability,
									delay,
									halt_on_match,
									random_balance,
									files,
								}
							);
						}

						else if current_sound.is_some() && name.local_name == "soundFile" {

							let mut path = PathBuf::from(file_path);
							path.pop();
							let mut is_playlist = false;
							let mut weight: usize = 0;		
							let mut volume: f32 = 0.0;	
							let mut random_balance: bool = false;
							let mut balance: f32 = 0.0;
							let mut delay: usize = 0;

							for attr in attributes {
								let attr_name = attr.name.local_name.as_str();
								match attr_name {
									"fileName" => path.push(attr.value),
									"weight" => {
										weight = attr.value.parse().unwrap();
									}
									"volumeAdjustment" => {
										volume = attr.value.parse().unwrap();
									}
									"randomBalance" => {
										if attr.value == "true" { 
											random_balance = true;
										}
									}
									"balanceAdjustment" => {
										balance = attr.value.parse().unwrap();
									}
									"delay" => {
										delay = attr.value.parse().unwrap();
									}
									"playlist" => {is_playlist = true;}
									_ => println!("Unknown sound-file value: {}", attr_name)
								}
							}
							let r#type = if is_playlist {
								let path_vec = parse_playlist(&path);
								SoundFileType::IsPlaylist(path_vec)
							} else {
								// test_file(&path);
								SoundFileType::IsPath(path)
							};
							let sound_file = SoundFile {
								r#type,
								weight,
								volume,
								random_balance,
								delay,
								balance,
							};
							current_sound.as_mut().unwrap()
								.files.push(sound_file);
						}
					},

					XmlEvent::EndElement{name} => {
						if current_sound.is_some() && name.local_name == "sound" {
							sounds.push( current_sound.take().unwrap() );
						}
					},

					_ => ()
				}
			}
		};

		visit_dir(sound_dir, &mut func);
		let channel_names = channels.keys().cloned().collect();
		ui_sender.send(UIMessage::ChannelNames(channel_names)).unwrap();

		println!("Finished loading!");
		Self {
			sounds,
			device,
			channels,
			total_volume: 1.0,
			concurency: 0,
			ui_sender,
			rng: thread_rng(),
		}
	}

	pub fn maintain(&mut self) {
		self.concurency = 0;
		for chn in self.channels.values_mut() {
			chn.maintain(&self.device, &mut self.rng, Some(&self.ui_sender));
			self.concurency += chn.len();
		}
	}

	pub fn set_volume(&mut self, channel_name: &str, volume: f32) {
		if channel_name == "all" {
			self.total_volume = volume;
			for channel in self.channels.values_mut() {
				channel.set_volume(channel.volume, self.total_volume);
			}
		}
		else if let Some(channel) = self.channels.get_mut(channel_name) {
			channel.set_volume(volume, self.total_volume);
		}
	}

	pub fn process_log(&mut self, log: &str) {
		println!("log: {}", log);

		let rng = &mut self.rng;

		for sound in self.sounds.iter() {
			if sound.pattern.is_match(log) {
				println!("--pattern: {}", sound.pattern.as_str());

				let mut can_play = true;
				if let Some(probability) = sound.probability {
					can_play &= probability < rng.next_u32() as usize;
				}
				if let Some(concurency) = sound.concurency {
					can_play &= self.concurency <= concurency;
				}

				if can_play {
					if let Some(chn) = &sound.channel {
						println!("--channel: {}", chn);
						let device = &self.device;
						let channel = self.channels.get_mut(chn).unwrap();
						let files = &sound.files;
						
						if let Some(is_loop_start) = sound.loop_attr {
							if is_loop_start {
								println!("----loop=start");
								channel.change_loop(device, sound.files.as_slice(), rng);
							} else {
								println!("----loop=stop");
								channel.change_loop(device, &[], rng);
								if !sound.files.is_empty() {
									channel.add_oneshot(device, files.choose(rng).unwrap(), rng);
								}
							}
						}
						else if !sound.files.is_empty() && channel.len() <= sound.concurency.unwrap_or(std::usize::MAX) {
							channel.add_oneshot(device, files.choose(rng).unwrap(), rng);
						}
					
					} else if !sound.files.is_empty() {
						let channel = self.channels.get_mut("misc").unwrap();
						if channel.len() <= sound.concurency.unwrap_or(std::usize::MAX) {
							channel.add_oneshot(&self.device, (&sound.files).choose(rng).unwrap(), rng);
						}
					}
				}

				if sound.halt_on_match {
					break;
				}
			}
		}
	}
}

fn parse_playlist(path: &Path) -> Vec<PathBuf> {
	lazy_static! {
		static ref M3U_PATTERN: Regex = Regex::new(
				r"#EXT[A-Z]*"
			).unwrap();
		static ref PLS_PATTERN: Regex = Regex::new(
				r"File.+=(.+)"
			).unwrap();
	}

	let parent_path = path.parent().unwrap();

	let mut path_vec = Vec::new();
	let mut f = File::open(path).unwrap();
	let buf = &mut String::new();
	let extension = path.extension().unwrap();
	if extension == "m3u" {
		f.read_to_string(buf).unwrap();
		for line in buf.lines() {
			if !M3U_PATTERN.is_match(line) {
				let mut path = PathBuf::from(parent_path);
				path.push(line);
				path_vec.push(path);
			}
		}
	}
	else if extension == "pls" {
		f.read_to_string(buf).unwrap();
		for line in buf.lines() {
			if let Some(caps) = PLS_PATTERN.captures(line) {
				let mut path = PathBuf::from(parent_path);
				path.push(&caps[0]);
				path_vec.push(path);
			}
		}
	}
	else {
		unreachable!("Playlist {:?} is not valid!", path)
	}

	// for path in path_vec.iter() {
	// 	test_file(path);
	// }
	path_vec
}

// fn test_file(path: &Path) {
// 	let f = File::open(path).unwrap();
// 	if let Err(e) = Decoder::new(f) {
// 		println!("file: {}\nerror: {}", e, path.to_string_lossy());
// 	}
// }
// file: Unrecognized format
// error: D:\MyProjects\Rust\soundsense-rs\soundpacks\battle\hit/punch/punch4.mp3
// error: D:\MyProjects\Rust\soundsense-rs\soundpacks\battle\hit/punch/punch4.mp3
// file: Unrecognized format
// error: D:\MyProjects\Rust\soundsense-rs\soundpacks\battle\hit/punch/punch4.mp3
// file: Unrecognized format
// error: D:\MyProjects\Rust\soundsense-rs\soundpacks\battle\hit/punch/punch4.mp3
// file: Unrecognized format
// error: D:\MyProjects\Rust\soundsense-rs\soundpacks\battle\hit/push/push5.mp3
// file: Unrecognized format
// error: D:\MyProjects\Rust\soundsense-rs\soundpacks\battle\hit/punch/punch4.mp3
// file: Unrecognized format
// error: D:\MyProjects\Rust\soundsense-rs\soundpacks\battle\hit/punch/punch4.mp3