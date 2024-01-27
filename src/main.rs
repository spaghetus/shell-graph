use std::path::PathBuf;

use eframe::egui::{CentralPanel, TopBottomPanel};
use rfd::FileDialog;
use shell_graph::Project;

struct App {
	pub project: Option<(Project, Option<PathBuf>)>,
}

impl eframe::App for App {
	fn update(&mut self, ctx: &eframe::egui::Context, frame: &mut eframe::Frame) {
		if let Some((project, path)) = &mut self.project {
			todo!();
		} else {
			self.project_picker(ctx);
		}
	}
}

impl App {
	fn project_picker(&mut self, ctx: &eframe::egui::Context) {
		CentralPanel::default().show(ctx, |ui| {
			ui.centered_and_justified(|ui| {
				ui.vertical(|ui| {
					ui.label("No file loaded!");
					if ui.button("Select a file").clicked() {
						if let Some(path) = FileDialog::new()
							.add_filter("Graph file", &["shgraph"])
							.pick_file()
						{
							let file = std::fs::read_to_string(&path).expect("Read failure");
							let project: Project =
								serde_yaml::from_str(&file).expect("Deserialize failure");
							self.project = Some((project, Some(path)))
						}
					}
				})
			})
		});
	}
}

fn main() {
	println!("Hello, world!");
}
