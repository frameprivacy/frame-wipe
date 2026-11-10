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
        (Lang::En, "ui.available") => "Available",
        (Lang::Fr, "ui.available") => "Disponible",
        (Lang::En, "ui.protected") => "Protected",
        (Lang::Fr, "ui.protected") => "Protege",
        (Lang::En, "ui.drives_summary") => "Drives on this computer",
        (Lang::Fr, "ui.drives_summary") => "Disques de cet ordinateur",
        (Lang::En, "ui.tips") => "Before you start",
        (Lang::Fr, "ui.tips") => "Avant de commencer",
        (Lang::En, "ui.tip1") => "Unmount the volume if you want to overwrite the whole device.",
        (Lang::Fr, "ui.tip1") => {
            "Demontez le volume si vous voulez ecraser le peripherique entier."
        }
        (Lang::En, "ui.tip2") => {
            "You will have to type the device name, then wait through a short countdown."
        }
        (Lang::Fr, "ui.tip2") => {
            "Vous devrez saisir le nom du peripherique, puis attendre un court compte a rebours."
        }
        (Lang::En, "ui.tip3") => {
            "An overwrite on an SSD cannot reach every remapped block. Read SAFETY.md."
        }
        (Lang::Fr, "ui.tip3") => {
            "Un ecrasement sur un SSD n'atteint pas tous les blocs remappes. Lisez SAFETY.md."
        }
        (Lang::En, "ui.overwrite") => "Overwrite…",
        (Lang::Fr, "ui.overwrite") => "Ecraser…",
        (Lang::En, "ui.estimate") => "Rough time estimate",
        (Lang::Fr, "ui.estimate") => "Estimation du temps",
        (Lang::En, "ui.estimate_note") => {
            "Assumes about 500 MiB/s. Actual time depends on the drive and the method."
        }
        (Lang::Fr, "ui.estimate_note") => {
            "Base sur environ 500 Mio/s. Le temps reel depend du disque et de la methode."
        }
        (Lang::En, "ui.serial") => "Serial",
        (Lang::Fr, "ui.serial") => "Numero de serie",
        (Lang::En, "ui.size") => "Size",
        (Lang::Fr, "ui.size") => "Taille",
        (Lang::En, "ui.path") => "Path",
        (Lang::Fr, "ui.path") => "Chemin",
        (Lang::En, "ui.type") => "Type",
        (Lang::Fr, "ui.type") => "Type",
        (Lang::En, "ui.none_available") => {
            "No overwritable drives. Protected disks are listed on the left, dimmed."
        }
        (Lang::Fr, "ui.none_available") => {
            "Aucun disque ecrasable. Les disques proteges sont listes a gauche, estompes."
        }
        (Lang::En, "ui.empty") => {
            "No drives were found. You may need to run this program as administrator."
        }
        (Lang::Fr, "ui.empty") => {
            "Aucun disque trouve. Il faudra peut-etre lancer ce programme en administrateur."
        }
        (Lang::En, "ui.protected_reason_system") => {
            "System disk. Refused unless you override this in the command-line tool."
        }
        (Lang::Fr, "ui.protected_reason_system") => {
            "Disque systeme. Refuse, sauf exception dans l'outil en ligne de commande."
        }
        (Lang::En, "ui.protected_reason_app") => "This is the disk Frame Wipe is running from.",
        (Lang::Fr, "ui.protected_reason_app") => "C'est le disque depuis lequel Frame Wipe tourne.",
        (Lang::En, "ui.protected_reason_raid") => {
            "RAID or LVM member. Wiping one leg of an array is easy to get wrong."
        }
        (Lang::Fr, "ui.protected_reason_raid") => {
            "Membre RAID ou LVM. Ecraser un seul element d'un ensemble est risque."
        }
        (Lang::En, "ui.unknown_model") => "Unknown model",
        (Lang::Fr, "ui.unknown_model") => "Modele inconnu",
        (Lang::En, "ui.no_serial") => "No serial",
        (Lang::Fr, "ui.no_serial") => "Pas de numero de serie",
        (Lang::En, "ui.report_title") => "Completed",
        (Lang::Fr, "ui.report_title") => "Termine",
        (Lang::En, "ui.back") => "Back to drives",
        (Lang::Fr, "ui.back") => "Retour aux disques",
        (Lang::En, "ui.mounted_hint") => {
            "This volume is mounted. Choose free space only, or unmount it first."
        }
        (Lang::Fr, "ui.mounted_hint") => {
            "Ce volume est monte. Choisissez l'espace libre, ou demontez-le d'abord."
        }
        (Lang::En, "ui.can_overwrite") => "can be overwritten",
        (Lang::Fr, "ui.can_overwrite") => "peuvent etre ecrases",
        (Lang::En, "ui.protected_count") => "protected",
        (Lang::Fr, "ui.protected_count") => "proteges",
        (Lang::En, "ui.working") => "Overwriting",
        (Lang::Fr, "ui.working") => "Ecrasement en cours",
        (Lang::En, "ui.confirm_name") => "Device name",
        (Lang::Fr, "ui.confirm_name") => "Nom du peripherique",
        _ => "??",
    }
}
