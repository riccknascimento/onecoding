// Evita a janela de console extra no Windows em release (inofensivo no macOS).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    oneagent_desktop::run();
}
