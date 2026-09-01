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

#[path = "optimizer/profile_mapping.rs"]
mod profile_mapping;
