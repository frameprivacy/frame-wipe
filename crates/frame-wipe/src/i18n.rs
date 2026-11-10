//! English and French strings for the CLI and GUI.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
}

impl Lang {
    pub fn parse(s: &str) -> Self {
        let s = s.trim().to_ascii_lowercase();
        if s.starts_with("fr") {
            Self::Fr
        } else {
            Self::En
        }
    }

    pub fn from_env() -> Self {
        for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(v) = std::env::var(key) {
                if !v.is_empty() && v != "C" && v != "C.UTF-8" {
                    return Self::parse(&v);
                }
            }
        }
        Self::En
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Fr => "fr",
        }
    }
}

pub fn t(lang: Lang, key: &str) -> &'static str {
    match (lang, key) {
        (Lang::En, "app.name") => "Frame Wipe",
        (Lang::Fr, "app.name") => "Frame Wipe",
        (Lang::En, "app.tagline") => "Overwrite disks, partitions, free space, or files.",
        (Lang::Fr, "app.tagline") => {
            "Ecraser un disque, une partition, l'espace libre ou des fichiers."
        }
        (Lang::En, "ui.devices") => "Devices",
        (Lang::Fr, "ui.devices") => "Peripheriques",
        (Lang::En, "ui.refresh") => "Refresh",
        (Lang::Fr, "ui.refresh") => "Actualiser",
        (Lang::En, "ui.method") => "Method",
        (Lang::Fr, "ui.method") => "Methode",
        (Lang::En, "ui.zeros") => "Zeros",
        (Lang::Fr, "ui.zeros") => "Zeros",
        (Lang::En, "ui.random") => "Random",
        (Lang::Fr, "ui.random") => "Aleatoire",
        (Lang::En, "ui.multi") => "Multi-pass",
        (Lang::Fr, "ui.multi") => "Plusieurs passes",
        (Lang::En, "ui.verify") => "Verify after write",
        (Lang::Fr, "ui.verify") => "Verifier apres l'ecriture",
        (Lang::En, "ui.dry_run") => "Dry run",
        (Lang::Fr, "ui.dry_run") => "Simulation",
        (Lang::En, "ui.scope") => "Scope",
        (Lang::Fr, "ui.scope") => "Portee",
        (Lang::En, "ui.whole") => "Whole device",
        (Lang::Fr, "ui.whole") => "Peripherique entier",
        (Lang::En, "ui.partition") => "Partition",
        (Lang::Fr, "ui.partition") => "Partition",
        (Lang::En, "ui.freespace") => "Free space only",
        (Lang::Fr, "ui.freespace") => "Espace libre seulement",
        (Lang::En, "ui.files") => "Files and folders",
        (Lang::Fr, "ui.files") => "Fichiers et dossiers",
        (Lang::En, "ui.start") => "Start",
        (Lang::Fr, "ui.start") => "Demarrer",
        (Lang::En, "ui.cancel") => "Cancel",
        (Lang::Fr, "ui.cancel") => "Annuler",
        (Lang::En, "ui.confirm_title") => "Type the device name to confirm",
        (Lang::Fr, "ui.confirm_title") => "Saisissez le nom du peripherique pour confirmer",
        (Lang::En, "ui.confirm_hint") => "This cannot be undone. Type the name exactly.",
        (Lang::Fr, "ui.confirm_hint") => "Cette action est definitive. Saisissez le nom exact.",
        (Lang::En, "ui.countdown") => "Starting in",
        (Lang::Fr, "ui.countdown") => "Demarrage dans",
        (Lang::En, "ui.system") => "SYSTEM",
        (Lang::Fr, "ui.system") => "SYSTEME",
        (Lang::En, "ui.mounted") => "MOUNTED",
        (Lang::Fr, "ui.mounted") => "MONTE",
        (Lang::En, "ui.app_disk") => "APP DISK",
        (Lang::Fr, "ui.app_disk") => "DISQUE APP",
        (Lang::En, "ui.raid") => "RAID/LVM",
        (Lang::Fr, "ui.raid") => "RAID/LVM",
        (Lang::En, "ui.in_use") => "IN USE",
        (Lang::Fr, "ui.in_use") => "EN USAGE",
        (Lang::En, "ui.language") => "Language",
        (Lang::Fr, "ui.language") => "Langue",
        (Lang::En, "ui.no_devices") => "No devices found. You may need to run as administrator.",
        (Lang::Fr, "ui.no_devices") => {
            "Aucun peripherique. Il faudra peut-etre lancer en administrateur."
        }
        (Lang::En, "ui.save_report") => "Save report",
        (Lang::Fr, "ui.save_report") => "Enregistrer le rapport",
        (Lang::En, "ui.done") => "Done",
        (Lang::Fr, "ui.done") => "Termine",
        (Lang::En, "ui.failed") => "Failed",
        (Lang::Fr, "ui.failed") => "Echec",
        (Lang::En, "ui.badge.ssd") => "SSD",
        (Lang::Fr, "ui.badge.ssd") => "SSD",
        _ => "??",
    }
}
