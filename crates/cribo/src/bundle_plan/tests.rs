//! Tests for bundle plan module

use super::*;
use crate::bundle_plan::builder::BundlePlanBuilder;

#[test]
fn test_bundle_plan_creation() {
    let plan = BundlePlan::new();
    assert!(plan.import_rewrites.is_empty());
    assert!(plan.module_metadata.is_empty());
    assert!(plan.live_items.is_empty());
    assert!(plan.hoisted_imports.is_empty());
}

#[test]
fn test_bundle_plan_builder() {
    let plan = BundlePlanBuilder::new().build();
    assert!(plan.import_rewrites.is_empty());
    assert!(plan.module_metadata.is_empty());
}

// Note: Full tests with ModuleId/ItemId will be added once
// we have proper constructors or test helpers for these types
