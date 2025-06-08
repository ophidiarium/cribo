#!/bin/bash

# Test script to validate wheel tag ordering fix logic
# This simulates the fix that will be applied in GitHub Actions

echo "🧪 Testing wheel tag ordering fix logic..."

# Create a test directory
TEST_DIR="test-wheels"
mkdir -p "$TEST_DIR"

# Create test wheel files with problematic names
touch "$TEST_DIR/cribo-1.0.0-py3-none-manylinux2014_aarch64.manylinux_2_17_aarch64.whl"
touch "$TEST_DIR/cribo-1.0.0-py3-none-manylinux2014_x86_64.manylinux_2_17_x86_64.whl"
touch "$TEST_DIR/cribo-1.0.0-py3-none-linux_aarch64.whl" # This one should not be changed

echo "📋 Original wheel files:"
ls -la "$TEST_DIR/"

echo ""
echo "🔧 Applying wheel tag ordering fix..."

# Apply the same fix logic as in the workflow
for wheel in "$TEST_DIR"/*.whl; do
    if [[ -f "$wheel" ]]; then
        filename=$(basename "$wheel")
        echo "Processing: $filename"

        # Check if this is a problematic manylinux wheel with multiple platform tags
        if [[ "$filename" =~ manylinux2014_aarch64\.manylinux_2_17_aarch64 ]]; then
            # Extract the correct sorted filename
            corrected_filename=$(echo "$filename" | sed 's/manylinux2014_aarch64\.manylinux_2_17_aarch64/manylinux_2_17_aarch64.manylinux2014_aarch64/')

            echo "  ❌ Found incorrectly ordered tags: $filename"
            echo "  ✅ Renaming to PEP 425 compliant: $corrected_filename"

            mv "$wheel" "$TEST_DIR/$corrected_filename"
        elif [[ "$filename" =~ manylinux2014_x86_64\.manylinux_2_17_x86_64 ]]; then
            # Handle x86_64 case if it exists
            corrected_filename=$(echo "$filename" | sed 's/manylinux2014_x86_64\.manylinux_2_17_x86_64/manylinux_2_17_x86_64.manylinux2014_x86_64/')

            echo "  ❌ Found incorrectly ordered tags: $filename"
            echo "  ✅ Renaming to PEP 425 compliant: $corrected_filename"

            mv "$wheel" "$TEST_DIR/$corrected_filename"
        else
            echo "  ✅ Wheel has correct tag ordering: $filename"
        fi
    fi
done

echo ""
echo "📋 Final wheel listing:"
ls -la "$TEST_DIR/"

echo ""
echo "🧹 Cleaning up test files..."
rm -rf "$TEST_DIR"

echo "✅ Test completed successfully!"
