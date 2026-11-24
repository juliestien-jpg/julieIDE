//! Patchs (diff unifié) : **prévu pour la v1, rien n'est codé ici.**
//!
//! Ce qui viendra, pour que l'API se prépare autour :
//!
//! - écrire un diff unifié compatible `git apply` à partir d'un
//!   [`crate::Diff`] calculé avec [`crate::DiffOptions::exact`] (les numéros de
//!   ligne et la marque « \ No newline at end of file » s'en déduisent) ;
//! - lire un diff unifié (en-têtes, blocs, fichiers créés, supprimés, renommés) ;
//! - appliquer un bloc avec décalage toléré et contexte flou ;
//! - rendre proprement les blocs refusés.
//!
//! Les fins de ligne et l'absence de saut de ligne final y seront significatives,
//! contrairement à l'affichage côte à côte.
