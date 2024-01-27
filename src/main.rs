use std::path::PathBuf;

use eframe::{
	egui::{CentralPanel, TopBottomPanel},
	NativeOptions,
};
use rfd::FileDialog;
use shell_graph::{NodeTemplate, Project};

struct App {
	pub project: Option<(Project, Option<PathBuf>)>,
}

impl eframe::App for App {
	fn update(&mut self, ctx: &eframe::egui::Context, frame: &mut eframe::Frame) {
		if let Some((project, path)) = &mut self.project {
			project.tick_processes();
			TopBottomPanel::top("top").show(ctx, |ui| {
				ui.horizontal(|ui| {
					if ui.button("Save").clicked() {
						if path.is_none() {
							*path = FileDialog::new()
								.add_filter("Graph file", &["shgraph"])
								.save_file();
						}
						if let Some(path) = &path {
							let data = serde_yaml::to_string(&project).expect("Serialize failure");
							if let Err(e) = std::fs::write(path, &data) {
								eprintln!("Write failed! Error {e}");
								eprintln!("Recovered document:\n{data}");
							}
						}
					}
					if ui.button("Kill").clicked() {
						project.kill_processes();
					}
					if ui.button("Clear").clicked() {
						project.graves.clear();
						project.output.clear();
					}
					if ui.button("Run").clicked() {
						project.start();
					}
					if !project.processes.is_empty() {
						ui.label(format!("{} running...", project.processes.len()));
					}
				});
			});
			CentralPanel::default().show(ctx, |ui| {
				let _ =
					project
						.graph_editor
						.draw_graph_editor(ui, NodeTemplate, &mut project.inner);
			});
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
					if ui.button("New file").clicked() {
						self.project = Some((Project::default(), None));
					}
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

#[tokio::main]
async fn main() {
	let app = App { project: None };

	eframe::run_native(
		"Shell Graph",
		NativeOptions::default(),
		Box::new(|_| Box::new(app)),
	);
}
