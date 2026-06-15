/// Trait that metric collection plugins must implement.
pub trait MetricPlugin: Send + Sync {
    /// Human-readable name for this plugin.
    fn name(&self) -> &str;

    /// Shell command to run on the remote host to collect raw data.
    fn collect_command(&self) -> &str;

    /// Parse the raw command output into a displayable string.
    /// Returns `None` if the output could not be parsed.
    fn parse(&self, raw: &str) -> Option<String>;

    /// Whether this plugin is currently enabled.
    fn enabled(&self) -> bool {
        true
    }
}

/// Registry that holds all registered metric plugins.
pub struct PluginRegistry {
    plugins: Vec<Box<dyn MetricPlugin>>,
}

impl PluginRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Register a new metric plugin.
    pub fn register(&mut self, plugin: Box<dyn MetricPlugin>) {
        self.plugins.push(plugin);
    }

    /// Return references to all currently-enabled plugins.
    pub fn enabled_plugins(&self) -> Vec<&dyn MetricPlugin> {
        self.plugins
            .iter()
            .filter(|p| p.enabled())
            .map(|p| p.as_ref())
            .collect()
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}
