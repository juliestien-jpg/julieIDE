//! Les étapes `ui <commande> [argument]` du mode scénario : piloter les vues Git,
//! le dialogue de fusion et la vue des tests comme le ferait un clic ou une
//! touche, sans passer par des coordonnées. Voir `docs/essais.md`.

use super::*;

fn side_of(text: &str) -> Result<Side, String> {
    match text {
        "left" | "gauche" => Ok(Side::Left),
        "right" | "droite" => Ok(Side::Right),
        other => Err(format!("côté inconnu : « {other} » (left ou right)")),
    }
}

fn number(text: &str) -> Result<usize, String> {
    text.trim()
        .parse::<usize>()
        .map_err(|_| format!("nombre attendu : « {text} »"))
}

fn on_off(text: &str) -> Result<bool, String> {
    match text.trim() {
        "on" | "oui" | "true" => Ok(true),
        "off" | "non" | "false" => Ok(false),
        other => Err(format!("`on` ou `off` attendu : « {other} »")),
    }
}

impl JulieStormApp {
    pub(super) fn ui_command(&mut self, command: &str, arg: &str) -> Result<(), String> {
        let arg = arg.trim();
        if let Some(result) = self.fs_dialog_command(command, arg) {
            return result;
        }
        if let Some(result) = self.nav_ui_command(command, arg) {
            return result;
        }
        match command {
            // ----- Éditeurs fractionnés ---------------------------------------------
            // `group.activate <n>` : le groupe n (base 1, ordre d'affichage) devient actif.
            "group.activate" => {
                let n = number(arg)?;
                let order = self.editors.group_order();
                let group = *order
                    .get(n.wrapping_sub(1))
                    .ok_or_else(|| format!("groupe {n} absent ({} groupe(s))", order.len()))?;
                self.editors.activate_group(group);
            }
            // `group.tab <groupe> <onglet>` : active un onglet d'un groupe (bases 1).
            "group.tab" => {
                let mut parts = arg.split_whitespace();
                let n = number(parts.next().unwrap_or(""))?;
                let tab = number(parts.next().unwrap_or(""))?;
                let order = self.editors.group_order();
                let group = *order
                    .get(n.wrapping_sub(1))
                    .ok_or_else(|| format!("groupe {n} absent"))?;
                let count = self
                    .editors
                    .groups
                    .iter()
                    .find(|g| g.id == group)
                    .map_or(0, |g| g.tabs.len());
                if tab == 0 || tab > count {
                    return Err(format!("onglet {tab} absent du groupe {n}"));
                }
                self.editors.activate_tab(group, tab - 1);
            }
            // `layout.persist on|off` : sauvegarde et restauration de la disposition dans
            // `.juliestorm/` (désactivées par défaut en mode scénario).
            "layout.persist" => self.layout_persist = on_off(arg)?,
            // ----- Git ----------------------------------------------------------
            "git.tab" => {
                self.tools.bottom = BottomTool::Git;
                self.tools.bottom_open = true;
                self.git.tab =
                    GitTab::parse(arg).ok_or("onglet inconnu : changes, log ou console")?;
            }
            "git.select" => {
                if !self.git.select_path(arg) {
                    return Err(format!("fichier absent de la liste Git : {arg}"));
                }
            }
            "git.multi" => {
                let wanted: Vec<String> = arg.split(',').map(|s| s.trim().to_owned()).collect();
                if !self.git.select_many(&wanted) {
                    return Err("fichier absent de la liste Git".to_owned());
                }
            }
            "git.check" | "git.uncheck" => {
                if !self.git.set_checked(arg, command == "git.check") {
                    return Err(format!("fichier absent de la liste Git : {arg}"));
                }
            }
            "git.message" => self.git.message = arg.replace("\\n", "\n"),
            "git.amend" => self.git.amend = on_off(arg)?,
            "git.commit" => {
                let intent = self
                    .git
                    .commit_intent()
                    .ok_or("commit impossible : message vide ou rien de coché")?;
                self.run_git_intents(vec![intent]);
            }
            "git.diff" => {
                let mut intents = Vec::new();
                self.git.open_diff_window(&mut intents);
                if intents.is_empty() {
                    return Err("aucun fichier sélectionné pour le diff".to_owned());
                }
                self.run_git_intents(intents);
            }
            "git.refresh" => {
                self.git.invalidate_diff();
                self.run_git_intents(vec![GitIntent::Refresh]);
            }
            "git.keep" => {
                let mut parts = arg.splitn(2, char::is_whitespace);
                let side = match parts.next() {
                    Some("ours" | "nous") => ConflictSide::Ours,
                    Some("theirs" | "eux") => ConflictSide::Theirs,
                    _ => return Err("`git.keep ours|theirs [fichier]`".to_owned()),
                };
                let wanted = parts.next().map(str::trim).filter(|w| !w.is_empty());
                if !self.git.conflicts_keep(side, wanted) {
                    return Err("aucun fichier en conflit correspondant".to_owned());
                }
            }
            "git.continue" => {
                let intent = self
                    .git
                    .commit_intent()
                    .ok_or("conclusion impossible : message vide ou conflits restants")?;
                self.run_git_intents(vec![intent]);
            }
            "git.abort" => {
                if !self.git.ask_abort() {
                    return Err("aucune opération en cours".to_owned());
                }
            }
            "git.log-select" => {
                let mut intents = Vec::new();
                let index = number(arg)?.saturating_sub(1);
                if !self.git.select_log(index, &mut intents) {
                    return Err(format!("commit {arg} absent du journal"));
                }
                self.run_git_intents(intents);
            }
            "git.commit-file" => {
                let mut intents = Vec::new();
                if !self
                    .git
                    .open_commit_file(number(arg)?.saturating_sub(1), &mut intents)
                {
                    return Err(format!("fichier {arg} absent du commit sélectionné"));
                }
                self.run_git_intents(intents);
            }
            "diff.close" => self.diff_window = None,

            // ----- Fusion -------------------------------------------------------
            "merge.open" => {
                let status = self.git.status().cloned().ok_or("état Git inconnu")?;
                let wanted = arg.replace('\\', "/");
                let file = status
                    .files
                    .iter()
                    .find(|f| f.repo_path == wanted)
                    .ok_or_else(|| format!("fichier absent de la liste Git : {arg}"))?;
                self.git.merge_loading = Some(file.path.clone());
                let path = file.path.clone();
                self.run_git_intents(vec![GitIntent::OpenMerge(path)]);
            }
            "merge.accept" | "merge.ignore" | "merge.both" => {
                let mut parts = arg.split_whitespace();
                let id = number(parts.next().unwrap_or(""))?;
                let side = side_of(parts.next().unwrap_or(""))?;
                let dialog = self.merge.as_mut().ok_or("aucune fusion ouverte")?;
                match command {
                    "merge.accept" => dialog.accept(id, side)?,
                    "merge.ignore" => dialog.ignore(id, side)?,
                    _ => dialog.accept_both(id, side)?,
                }
            }
            "merge.apply" => {
                let scope = match arg {
                    "all" | "tous" => ApplyScope::All,
                    "left" | "gauche" => ApplyScope::Left,
                    "right" | "droite" => ApplyScope::Right,
                    _ => return Err("`merge.apply all|left|right`".to_owned()),
                };
                self.merge
                    .as_mut()
                    .ok_or("aucune fusion ouverte")?
                    .apply(scope);
            }
            "merge.next" | "merge.prev" => {
                self.merge
                    .as_mut()
                    .ok_or("aucune fusion ouverte")?
                    .step(command == "merge.next");
            }
            "merge.undo" => self.merge.as_mut().ok_or("aucune fusion ouverte")?.undo(),
            "merge.redo" => self.merge.as_mut().ok_or("aucune fusion ouverte")?.redo(),
            "merge.edit" => {
                let (line, text) = arg.split_once(char::is_whitespace).unwrap_or((arg, ""));
                let line = number(line)?.saturating_sub(1);
                self.merge
                    .as_mut()
                    .ok_or("aucune fusion ouverte")?
                    .edit_line(line, text.trim_start())?;
            }
            "merge.focus" => {
                let dialog = self.merge.as_mut().ok_or("aucune fusion ouverte")?;
                dialog.focus_center();
                if let Ok(line) = number(arg) {
                    dialog.goto_line(line.saturating_sub(1) as u32);
                }
            }
            "merge.save" => {
                let mut intents = Vec::new();
                self.merge
                    .as_mut()
                    .ok_or("aucune fusion ouverte")?
                    .request_save(&mut intents);
                self.run_merge_intents(intents);
            }
            "merge.close" => {
                let mut intents = Vec::new();
                self.merge
                    .as_mut()
                    .ok_or("aucune fusion ouverte")?
                    .request_close(&mut intents);
                self.run_merge_intents(intents);
            }

            // ----- Réponses aux questions ---------------------------------------
            "confirm" | "cancel" => {
                let yes = command == "confirm";
                if let Some(dialog) = &mut self.merge {
                    if dialog.has_question() {
                        let mut intents = Vec::new();
                        dialog.answer(yes, &mut intents);
                        self.run_merge_intents(intents);
                        return Ok(());
                    }
                }
                if self.git.confirm.is_some() {
                    let mut intents = Vec::new();
                    self.git.answer_confirm(yes, &mut intents);
                    self.run_git_intents(intents);
                } else if self.tests.confirm_restart {
                    self.tests.confirm_restart = false;
                    if yes {
                        self.run_tests(true);
                    } else {
                        self.tests.pending_run = None;
                    }
                } else {
                    self.git.dismiss_error();
                }
            }

            // ----- Tests --------------------------------------------------------
            "tests.run" => self.start_run(RunRequest::all(), false),
            "tests.stop" => self.stop_tests(),
            "tests.select" => {
                self.tools.bottom = BottomTool::Tests;
                self.tools.bottom_open = true;
                if !self.tests.select_label(arg) {
                    return Err(format!("nœud de test introuvable : {arg}"));
                }
            }
            "tests.toggle" => self.tests.toggle_selected(),
            "tests.passed" => self.tests.show_passed = on_off(arg)?,
            "tests.compare" => {
                let intent = self
                    .tests
                    .compare_intent()
                    .ok_or("le nœud choisi n'a pas d'attendu / obtenu")?;
                self.run_tests_intents(vec![intent]);
            }
            "tests.link" => {
                let location = self
                    .tests
                    .link(number(arg)?)
                    .ok_or_else(|| format!("lien {arg} absent de la console"))?;
                self.run_tests_intents(vec![TestsIntent::Open(location)]);
            }
            other => {
                if let Some(result) = self.ui_command_settings(other, arg) {
                    return result;
                }
                match self.ui_command_v1(other, arg) {
                    Some(result) => return result,
                    None => return Err(format!("commande ui inconnue : « {other} »")),
                }
            }
        }
        Ok(())
    }
}
