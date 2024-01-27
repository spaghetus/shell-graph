use std::{
	collections::HashMap,
	fs::Permissions,
	io::Read,
	ops::{Deref, DerefMut},
	os::unix::fs::PermissionsExt,
	path::PathBuf,
	process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio},
	str::FromStr,
	sync::{Arc, RwLock},
};

use eframe::{
	egui::{Response, ScrollArea, Ui},
	epaint::Color32,
};
use egui_node_graph::{
	DataTypeTrait, Graph, GraphEditorState, GraphNodeWidget, InputId, NodeDataTrait, NodeId,
	NodeResponse, NodeTemplateIter, NodeTemplateTrait, OutputId, UserResponseTrait,
	WidgetValueTrait,
};
use nonblock::NonBlockingReader;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct UserResponse(pub Response);

impl UserResponseTrait for UserResponse {}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct NodeTemplate;

impl NodeTemplateTrait for NodeTemplate {
	type NodeData = Node;

	type DataType = PipeKind;

	type ValueType = NoValues;

	type UserState = ProjectInner;

	fn node_finder_label(&self, user_state: &mut Self::UserState) -> std::borrow::Cow<str> {
		std::borrow::Cow::Borrowed("Script")
	}

	fn node_graph_label(&self, user_state: &mut Self::UserState) -> String {
		String::from("Script")
	}

	fn user_data(&self, user_state: &mut Self::UserState) -> Self::NodeData {
		Node {
			script: Default::default(),
		}
	}

	fn build_node(
		&self,
		graph: &mut Graph<Self::NodeData, Self::DataType, Self::ValueType>,
		user_state: &mut Self::UserState,
		node_id: NodeId,
	) {
		// TODO do this
	}
}

impl NodeTemplateIter for NodeTemplate {
	type Item = Self;

	fn all_kinds(&self) -> Vec<Self::Item> {
		vec![Self]
	}
}

#[derive(Serialize, Deserialize, Default, Clone, Copy)]
pub struct NoValues;

impl WidgetValueTrait for NoValues {
	type Response = UserResponse;

	type UserState = ProjectInner;

	type NodeData = Node;

	fn value_widget(
		&mut self,
		param_name: &str,
		node_id: NodeId,
		ui: &mut Ui,
		user_state: &mut Self::UserState,
		node_data: &Self::NodeData,
	) -> Vec<Self::Response> {
		vec![UserResponse(ui.label("Should never appear"))]
	}
}

#[derive(Serialize, Deserialize, Default)]
pub struct Project {
	#[serde(rename = "graph_state")]
	pub graph_editor: GraphEditorState<Node, PipeKind, NoValues, NodeTemplate, ProjectInner>,
	#[serde(flatten)]
	pub inner: ProjectInner,
}

impl Clone for Project {
	fn clone(&self) -> Self {
		Self {
			graph_editor: self.graph_editor.clone(),
			inner: Default::default(),
		}
	}
}

impl Deref for Project {
	type Target = ProjectInner;

	fn deref(&self) -> &Self::Target {
		&self.inner
	}
}

impl DerefMut for Project {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.inner
	}
}

#[derive(Serialize, Deserialize, Default)]
pub struct ProjectInner {
	#[serde(skip)]
	pub processes: HashMap<NodeId, Process>,
	#[serde(skip)]
	pub graves: HashMap<NodeId, ExitStatus>,
	pub output: HashMap<NodeId, (Vec<u8>, Vec<u8>)>,
}

impl Clone for ProjectInner {
	fn clone(&self) -> Self {
		Self::default()
	}
}

impl Project {
	pub fn graph(&self) -> &Graph<Node, PipeKind, NoValues> {
		&self.graph_editor.graph
	}

	pub fn start(&mut self) {
		self.graves.clear();
		self.output.clear();
		let (inputs, outputs) = self
			.graph()
			.iter_connections()
			.map(|(input, output)| ((input, output), Arc::new(Pipe::default())))
			.fold(
				(
					HashMap::<InputId, Vec<Arc<Pipe>>>::new(),
					HashMap::<OutputId, Vec<Arc<Pipe>>>::new(),
				),
				|(mut inputs, mut outputs), ((input, output), pipe)| {
					inputs.entry(input).or_default().push(pipe.clone());
					outputs.entry(output).or_default().push(pipe.clone());
					(inputs, outputs)
				},
			);
		let scripts: HashMap<NodeId, Arc<Script>> = self
			.graph()
			.iter_nodes()
			.filter_map(|node| self.graph().nodes.get(node))
			.map(|node| (node.id, Arc::new((&node.user_data).into())))
			.collect();

		#[allow(clippy::type_complexity)]
		let process_patterns: HashMap<
			NodeId,
			(
				Arc<Script>,
				HashMap<&str, Vec<Arc<Pipe>>>,
				HashMap<&str, Vec<Arc<Pipe>>>,
			),
		> = scripts
			.into_iter()
			.map(|(id, script)| {
				let node = self.graph().nodes.get(id).unwrap();
				let inputs: HashMap<_, _> = node
					.inputs
					.iter()
					.map(|(name, id)| (name.as_str(), inputs.get(id).cloned().unwrap_or_default()))
					.collect();
				let outputs: HashMap<_, _> = node
					.outputs
					.iter()
					.map(|(name, id)| (name.as_str(), outputs.get(id).cloned().unwrap_or_default()))
					.collect();
				(id, (script, inputs, outputs))
			})
			.collect();

		let processes = process_patterns
			.into_iter()
			.map(|(id, (script, inputs, outputs))| {
				let env = inputs
					.into_iter()
					.map(|(name, pipes)| (format!("IN_{name}"), pipes))
					.chain(
						outputs
							.into_iter()
							.map(|(name, pipes)| (format!("OUT_{name}"), pipes)),
					)
					.map(|(name, pipes)| {
						(
							name,
							(
								pipes.clone(),
								pipes
									.into_iter()
									.map(|pipe| pipe.0.to_string_lossy().to_string())
									.collect::<Vec<_>>()
									.join(","),
							),
						)
					})
					.collect::<HashMap<_, _>>();
				let mut command = Command::new(&script.0);
				command.stdout(Stdio::piped());
				command.stderr(Stdio::piped());
				let mut pipes = vec![];
				for (key, (mut pipes_, value)) in env {
					pipes.append(&mut pipes_);
					command.env(key, value);
				}
				let mut child = dbg!(command).spawn().expect("Spawn failure");
				(
					id,
					Process {
						script,
						pipes,
						stdout: NonBlockingReader::from_fd(child.stdout.take().unwrap()).unwrap(),
						stderr: NonBlockingReader::from_fd(child.stderr.take().unwrap()).unwrap(),
						child,
					},
				)
			})
			.collect::<HashMap<_, _>>();
		self.processes = processes;
	}

	pub fn kill_processes(&mut self) {
		self.processes.clear();
	}

	pub fn tick_processes(&mut self) {
		self.processes = std::mem::take(&mut self.processes)
			.into_iter()
			.filter_map(|(id, mut process)| {
				let (output, error) = self.output.entry(id).or_default();
				process.stdout.read_available(output).expect("Read failure");
				process.stderr.read_available(error).expect("Read failure");
				if let Some(status) = process.child.try_wait().expect("Wait failure") {
					self.graves.insert(id, status);
					None
				} else {
					Some((id, process))
				}
			})
			.collect();
	}
}

#[derive(Serialize, Deserialize)]
pub struct Node {
	pub script: RwLock<String>,
}

impl From<&Node> for Script {
	fn from(value: &Node) -> Self {
		Script::from(value.script.read().unwrap().as_str())
	}
}

impl Clone for Node {
	fn clone(&self) -> Self {
		Self {
			script: self.script.read().unwrap().to_string().into(),
		}
	}
}

impl NodeDataTrait for Node {
	type Response = UserResponse;

	type UserState = ProjectInner;

	type DataType = PipeKind;

	type ValueType = NoValues;

	fn bottom_ui(
		&self,
		ui: &mut eframe::egui::Ui,
		node_id: NodeId,
		graph: &Graph<Self, Self::DataType, Self::ValueType>,
		user_state: &mut Self::UserState,
	) -> Vec<egui_node_graph::NodeResponse<Self::Response, Self>>
	where
		Self::Response: egui_node_graph::UserResponseTrait,
	{
		let node = graph
			.nodes
			.get(node_id)
			.expect("Tried to render missing node");
		let output = user_state.output.get(&node_id);
		let grave = user_state.graves.get(&node_id);
		vec![NodeResponse::User(UserResponse(
			ui.vertical(|ui| {
				ui.text_edit_multiline(&mut *node.user_data.script.write().unwrap());
				if let Some(status) = grave {
					ui.label(format!("STATUS: {status}"));
				}
				if let Some((stdout, stderr)) = output {
					ui.label("STDOUT:");
					ScrollArea::new([false, true])
						.id_source(("stdout", node_id))
						.auto_shrink([false, false])
						.max_height(150.0)
						.max_width(150.0)
						.show(ui, |ui| {
							ui.label(String::from_utf8_lossy(stdout));
						});
					ui.label("STDERR:");
					ScrollArea::new([false, true])
						.id_source(("stderr", node_id))
						.auto_shrink([false, false])
						.max_height(150.0)
						.max_width(150.0)
						.show(ui, |ui| {
							ui.label(String::from_utf8_lossy(stderr));
						});
				}
				if user_state.processes.contains_key(&node_id) {
					ui.spinner();
				}
			})
			.response,
		))]
	}
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
pub enum PipeKind {
	Single,
	Many,
}

impl DataTypeTrait<ProjectInner> for PipeKind {
	fn data_type_color(&self, _user_state: &mut ProjectInner) -> eframe::egui::Color32 {
		match self {
			PipeKind::Single => Color32::GREEN,
			PipeKind::Many => Color32::DARK_GREEN,
		}
	}

	fn name(&self) -> std::borrow::Cow<str> {
		todo!()
	}
}

pub struct Process {
	pub script: Arc<Script>,
	pub pipes: Vec<Arc<Pipe>>,
	pub child: Child,
	pub stdout: NonBlockingReader<ChildStdout>,
	pub stderr: NonBlockingReader<ChildStderr>,
}

#[derive(Debug)]
pub struct Script(pub PathBuf);
impl From<&str> for Script {
	fn from(value: &str) -> Self {
		let base = dirs::runtime_dir().unwrap_or_else(|| PathBuf::from_str("/tmp").unwrap());
		let base = base.join("shell-graph");
		std::fs::create_dir_all(&base).expect("Mkdir failure");
		let file = base.join(Uuid::new_v4().to_string());
		std::fs::write(&file, value).expect("Write failure");
		std::fs::set_permissions(&file, Permissions::from_mode(0o744)).expect("Chmod failure");
		Self(file)
	}
}
impl Drop for Script {
	fn drop(&mut self) {
		println!("Dropping script!");
		std::fs::remove_file(&self.0).expect("Cleanup failure");
	}
}

#[derive(Debug)]
pub struct Pipe(pub PathBuf);
impl Default for Pipe {
	fn default() -> Self {
		let base = dirs::runtime_dir().unwrap_or_else(|| PathBuf::from_str("/tmp").unwrap());
		let base = base.join("shell-graph");
		std::fs::create_dir_all(&base).expect("Mkdir failure");
		let file = base.join(Uuid::new_v4().to_string());
		nix::unistd::mkfifo(&file, nix::sys::stat::Mode::S_IRWXU).expect("Mkfifo failure");
		Self(file)
	}
}
impl Drop for Pipe {
	fn drop(&mut self) {
		std::fs::remove_file(&self.0).expect("Cleanup failure");
	}
}

#[test]
fn test_basic_graph() {
	let mut project = Project::default();

	let emitter = project.graph().add_node(
		"Node 1".to_string(),
		Node {
			script: "#!/usr/bin/env bash
			(echo no; sleep 10s; echo hello, world!; echo no) > $OUT_out
			"
			.to_string(),
		},
		|graph, id| {},
	);
	let output = project
		.graph()
		.add_output_param(emitter, "out".to_string(), PipeKind::Single);

	let receiver = project.graph().add_node(
		"Node 2".to_string(),
		Node {
			script: "#!/usr/bin/env bash
			cat $IN_in | grep '!'
			echo OK
			"
			.to_string(),
		},
		|graph, id| {},
	);
	let input = project.graph().add_input_param(
		receiver,
		"in".to_string(),
		PipeKind::Single,
		(),
		egui_node_graph::InputParamKind::ConnectionOnly,
		false,
	);

	project.graph().add_connection(output, input);

	project.start();
	while !project.processes.is_empty() {
		project.tick_processes();
		std::thread::sleep(std::time::Duration::from_millis(100));
	}
	println!("{:#?}", project.output);
}
