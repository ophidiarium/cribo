//! Tests for the transformation context module

use std::path::PathBuf;

use super::*;

#[test]
fn test_create_node_index() {
    let ctx = TransformationContext::new();

    let idx1 = ctx.create_node_index();
    let idx2 = ctx.create_node_index();

    // Indices should be sequential
    assert_eq!(idx1.load().as_usize(), 0);
    assert_eq!(idx2.load().as_usize(), 1);
}

#[test]
fn test_record_copy_transformation() {
    let mut ctx = TransformationContext::new();

    let original_module = PathBuf::from("original.py");
    let original_idx = AtomicNodeIndex::from(42).load();
    let transformed_node = AtomicNodeIndex::dummy();

    let transformed_idx = ctx.record_copy(original_module.clone(), original_idx, &transformed_node);

    // Should have recorded the mapping
    assert_eq!(
        ctx.node_mappings
            .get_transformed(&original_module, original_idx),
        Some(transformed_idx)
    );

    // Should have recorded the transformation
    assert_eq!(ctx.transformations.len(), 1);
    assert_eq!(
        ctx.transformations[0].transformation_type,
        TransformationType::DirectCopy
    );
    assert_eq!(
        ctx.transformations[0].original,
        Some((original_module, original_idx))
    );
    assert_eq!(ctx.transformations[0].transformed, transformed_idx);
}

#[test]
fn test_record_import_rewrite() {
    let mut ctx = TransformationContext::new();

    let original_module = PathBuf::from("imports.py");
    let original_idx = AtomicNodeIndex::from(10).load();
    let transformed_node = AtomicNodeIndex::dummy();

    let transformation_type = TransformationType::ImportRewritten {
        from_module: "old_module".to_string(),
        to_module: "new_module".to_string(),
    };

    let _transformed_idx = ctx.record_transformation(
        Some((original_module.clone(), original_idx)),
        &transformed_node,
        transformation_type.clone(),
    );

    // Should have recorded the transformation with correct type
    assert_eq!(ctx.transformations.len(), 1);
    assert_eq!(
        ctx.transformations[0].transformation_type,
        transformation_type
    );
}

#[test]
fn test_create_new_node() {
    let mut ctx = TransformationContext::new();

    let node_idx = ctx.create_new_node("Test node creation".to_string());

    // Should have a valid index
    let idx_value = node_idx.load().as_usize();
    assert!(idx_value < usize::MAX);

    // Should have recorded the transformation
    assert_eq!(ctx.transformations.len(), 1);
    match &ctx.transformations[0].transformation_type {
        TransformationType::NewNode { reason } => {
            assert_eq!(reason, "Test node creation");
        }
        _ => panic!("Expected NewNode transformation type"),
    }
}

#[test]
fn test_transformation_stats() {
    let mut ctx = TransformationContext::new();

    // Create various transformations
    ctx.create_new_node("Node 1".to_string());
    ctx.create_new_node("Node 2".to_string());

    let module = PathBuf::from("test.py");
    ctx.record_copy(
        module.clone(),
        AtomicNodeIndex::from(1).load(),
        &AtomicNodeIndex::dummy(),
    );
    ctx.record_copy(
        module.clone(),
        AtomicNodeIndex::from(2).load(),
        &AtomicNodeIndex::dummy(),
    );
    ctx.record_copy(
        module.clone(),
        AtomicNodeIndex::from(3).load(),
        &AtomicNodeIndex::dummy(),
    );

    ctx.record_transformation(
        None,
        &AtomicNodeIndex::dummy(),
        TransformationType::ImportRewritten {
            from_module: "old".to_string(),
            to_module: "new".to_string(),
        },
    );

    ctx.record_transformation(
        None,
        &AtomicNodeIndex::dummy(),
        TransformationType::GlobalsReplaced,
    );

    ctx.record_transformation(
        None,
        &AtomicNodeIndex::dummy(),
        TransformationType::ModuleWrapped {
            module_name: "test_module".to_string(),
        },
    );

    let stats = ctx.get_stats();

    assert_eq!(stats.total_transformations, 8);
    assert_eq!(stats.new_nodes, 2);
    assert_eq!(stats.direct_copies, 3);
    assert_eq!(stats.imports_rewritten, 1);
    assert_eq!(stats.globals_replaced, 1);
    assert_eq!(stats.modules_wrapped, 1);
    assert_eq!(stats.dead_code_eliminated, 0);
    assert_eq!(stats.nodes_merged, 0);
}

#[test]
fn test_get_transformation() {
    let mut ctx = TransformationContext::new();

    let module = PathBuf::from("test.py");
    let original_idx = AtomicNodeIndex::from(5).load();
    let transformed_node = AtomicNodeIndex::dummy();

    let transformed_idx = ctx.record_copy(module.clone(), original_idx, &transformed_node);

    // Should be able to retrieve the transformation
    let transformation = ctx.get_transformation(transformed_idx);
    assert!(transformation.is_some());

    let trans = transformation.expect("transformation should exist");
    assert_eq!(trans.original, Some((module, original_idx)));
    assert_eq!(trans.transformed, transformed_idx);
    assert_eq!(trans.transformation_type, TransformationType::DirectCopy);
}

#[test]
fn test_multiple_transformations() {
    let mut ctx = TransformationContext::new();

    // Simulate transforming multiple nodes from different modules
    let module1 = PathBuf::from("module1.py");
    let module2 = PathBuf::from("module2.py");

    // Transform nodes from module1
    for i in 0..5 {
        ctx.record_copy(
            module1.clone(),
            AtomicNodeIndex::from(i).load(),
            &AtomicNodeIndex::dummy(),
        );
    }

    // Transform nodes from module2 with different transformation types
    ctx.record_transformation(
        Some((module2.clone(), AtomicNodeIndex::from(0).load())),
        &AtomicNodeIndex::dummy(),
        TransformationType::ImportRewritten {
            from_module: "old_pkg".to_string(),
            to_module: "new_pkg".to_string(),
        },
    );

    ctx.record_transformation(
        Some((module2.clone(), AtomicNodeIndex::from(1).load())),
        &AtomicNodeIndex::dummy(),
        TransformationType::GlobalsReplaced,
    );

    // Create some new nodes
    ctx.create_new_node("Helper function for bundling".to_string());
    ctx.create_new_node("Namespace object creation".to_string());

    let stats = ctx.get_stats();
    assert_eq!(stats.total_transformations, 9);
    assert_eq!(stats.direct_copies, 5);
    assert_eq!(stats.imports_rewritten, 1);
    assert_eq!(stats.globals_replaced, 1);
    assert_eq!(stats.new_nodes, 2);
}
