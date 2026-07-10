use super::{
    console::ConsoleState, dom_inspector::DomInspectorState, network::NetworkTabState,
    panel::DeveloperPanelState, tabs::DeveloperToolTab,
};

#[derive(Debug, Clone, Default)]
pub struct DeveloperToolsState {
    pub panel: DeveloperPanelState,
    pub console: ConsoleState,
    pub dom: DomInspectorState,
    pub network: NetworkTabState,
}

impl DeveloperToolsState {
    pub fn set_active_tab(&mut self, active_tab: DeveloperToolTab) {
        self.panel.set_active_tab(active_tab);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_tab_is_preserved_when_panel_closes() {
        let mut tools = DeveloperToolsState::default();
        tools.set_active_tab(DeveloperToolTab::Network);
        tools.panel.close();
        assert_eq!(tools.panel.active_tab, DeveloperToolTab::Network);
    }
}
