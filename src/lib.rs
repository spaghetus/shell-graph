use std::{
	collections::HashMap,
	fs::Permissions,
	io::Read,
	os::unix::fs::PermissionsExt,
	path::PathBuf,
	process::{Child, Command, ExitStatus, Stdio},
	str::FromStr,
	sync::Arc,
};

use egui_node_graph::{Graph, InputId, NodeId, OutputId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Default)]
pub struct Project {
	pub graph: Graph<Node, PipeKind, ()>,
	#[serde(skip)]
	pub processes: HashMap<NodeId, Process>,
	#[serde(skip)]
	pub graves: HashMap<NodeId, ExitStatus>,
	pub output: HashMap<NodeId, (String, String)>,
}

impl Project {
	pub fn start(&mut self) {
		self.graves.clear();
		self.output.clear();
		let (inputs, outputs) = self
			.graph
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
			.graph
			.iter_nodes()
			.filter_map(|node| self.graph.nodes.get(node))
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
				let node = self.graph.nodes.get(id).unwrap();
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
				let child = command.spawn().expect("Spawn failure");
				(
					id,
					Process {
						script,
						pipes,
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
				if let Some(stdout) = &mut process.child.stdout {
					stdout.read_to_string(output).expect("Read failure");
				}
				if let Some(stderr) = &mut process.child.stderr {
					stderr.read_to_string(error).expect("Read failure");
				}
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
	pub script: String,
}
impl From<&Node> for Script {
	fn from(value: &Node) -> Self {
		Script::from(value.script.as_str())
	}
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
pub enum PipeKind {
	Single,
	Many,
}

#[derive(Debug)]
pub struct Process {
	pub script: Arc<Script>,
	pub pipes: Vec<Arc<Pipe>>,
	pub child: Child,
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

	let emitter = project.graph.add_node(
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
		.graph
		.add_output_param(emitter, "out".to_string(), PipeKind::Single);

	let receiver = project.graph.add_node(
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
	let input = project.graph.add_input_param(
		receiver,
		"in".to_string(),
		PipeKind::Single,
		(),
		egui_node_graph::InputParamKind::ConnectionOnly,
		false,
	);

	project.graph.add_connection(output, input);

	project.start();
	while !project.processes.is_empty() {
		project.tick_processes();
		std::thread::sleep(std::time::Duration::from_millis(100));
	}
	println!("{:#?}", project.output);
}
