//! Résolution automatique des conflits simples : **prévue pour la v1, rien n'est
//! codé ici.**
//!
//! Chaque règle sera un sous-module à part, avec ses tests — y compris les cas où
//! elle doit s'abstenir :
//!
//! - `disjoint_edits` : les deux côtés modifient la même ligne à des endroits
//!   disjoints ;
//! - `identical_additions` : les deux côtés ajoutent les mêmes lignes ;
//! - `whitespace_only` : un côté ne change que des espaces.
//!
//! Une règle recevra un bloc [`crate::BlockKind::Conflict`] (base, nôtre, leur) et
//! rendra soit un texte résolu, soit « je m'abstiens ». **Dans le doute, c'est un
//! conflit** : aucune règle ne s'appliquera sans que l'utilisatrice l'ait demandé,
//! et [`crate::MergeSession::apply_non_conflicting`] n'y fera jamais appel.
