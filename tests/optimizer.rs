#[path = "optimizer/scalar.rs"]
mod scalar;

#[path = "optimizer/preservation.rs"]
mod preservation;

#[path = "support/mod.rs"]
mod support;

#[path = "support/generated.rs"]
mod generated;

#[path = "optimizer/alias_effects.rs"]
mod alias_effects;

#[path = "optimizer/kir_o1.rs"]
mod kir_o1;

#[path = "optimizer/kir_o2.rs"]
mod kir_o2;

#[path = "optimizer/kir_o3.rs"]
mod kir_o3;

#[path = "optimizer/vector_plan.rs"]
mod vector_plan;

#[path = "optimizer/transaction.rs"]
mod transaction;

#[path = "optimizer/specialization.rs"]
mod specialization;

#[path = "optimizer/unroll.rs"]
mod unroll;

#[path = "optimizer/slp.rs"]
mod slp;

#[path = "optimizer/vectorize.rs"]
mod vectorize;

#[path = "optimizer/wasm_affine_checker.rs"]
mod wasm_affine_checker;

#[path = "optimizer/wasm_affine.rs"]
mod wasm_affine;

#[path = "optimizer/stencil_peel.rs"]
mod stencil_peel;

#[path = "optimizer/interior_normalize.rs"]
mod interior_normalize;

#[path = "optimizer/invariant_load.rs"]
mod invariant_load;

#[path = "optimizer/normalization_unswitch.rs"]
mod normalization_unswitch;

#[path = "optimizer/polynomial_uf.rs"]
mod polynomial_uf;

#[path = "optimizer/matmul_uf.rs"]
mod matmul_uf;

#[path = "optimizer/runtime_unroll.rs"]
mod runtime_unroll;

#[path = "optimizer/stencil_vector.rs"]
mod stencil_vector;

#[path = "optimizer/decision_tree.rs"]
mod decision_tree;

#[path = "optimizer/decision_tree_checker.rs"]
mod decision_tree_checker;

#[path = "optimizer/vector_alias_predicate.rs"]
mod vector_alias_predicate;

#[path = "optimizer/profile_mapping.rs"]
mod profile_mapping;

#[path = "optimizer/pgo.rs"]
mod pgo;

#[path = "optimizer/multiversion.rs"]
mod multiversion;

#[path = "optimizer/modular_affine.rs"]
mod modular_affine;
