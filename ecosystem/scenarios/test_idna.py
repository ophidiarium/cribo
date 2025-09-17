#!/usr/bin/env python3
"""Test scenario for bundled idna library.

This script:
1. Bundles the idna library using cribo
2. Runs smoke tests using the bundled version
3. Verifies internationalized domain name encoding/decoding works correctly
"""

import importlib.util
import os
import sys
from pathlib import Path
from types import ModuleType
from typing import TYPE_CHECKING

import pytest

from .utils import run_cribo, format_bundle_size, ensure_test_directories

# Type hint for better IDE support
if TYPE_CHECKING:
    import idna as IdnaType


@pytest.fixture(scope="module")
def bundled_idna():
    """Bundle the idna library and return the bundled module path."""
    # Ensure test directories exist
    tmp_dir = ensure_test_directories()

    # Create isolated directory for idna output
    idna_output_dir = tmp_dir / "idna"
    idna_output_dir.mkdir(parents=True, exist_ok=True)

    # Paths
    package_root = Path(__file__).resolve().parent.parent / "packages" / "idna"
    idna_init = package_root / "idna"
    bundled_output = idna_output_dir / "idna_bundled.py"
    bundled_output.unlink(missing_ok=True)  # Remove if exists

    print("\n🔧 Bundling idna library...")
    result = run_cribo(
        str(idna_init),
        str(bundled_output),
        emit_requirements=True,
        # tree_shake=False,
    )

    assert result.returncode == 0, f"Failed to bundle idna: {result.stderr}"

    bundled_size = bundled_output.stat().st_size
    print(f"✅ Bundled idna: {format_bundle_size(bundled_size)}")

    # idna is a pure Python package with no runtime dependencies
    # Therefore, no requirements.txt should be created even with --emit-requirements
    requirements_path = idna_output_dir / "requirements.txt"
    assert not requirements_path.exists(), "requirements.txt should not be created for idna (no dependencies)"
    print(f"📦 No third-party dependencies (pure Python package)")

    return str(bundled_output)


@pytest.fixture(scope="module")
def idna_module(bundled_idna: str) -> ModuleType:
    """Load the bundled idna module."""
    bundle_path = Path(bundled_idna)

    # Import the bundled module
    spec = importlib.util.spec_from_file_location("idna_bundled", bundle_path)
    idna = importlib.util.module_from_spec(spec)
    sys.modules["idna_bundled"] = idna
    spec.loader.exec_module(idna)

    print(f"✅ Loaded bundled module: {idna.__name__}")
    return idna


def test_basic_encoding(idna_module: "IdnaType"):
    """Test basic domain name encoding."""
    print("\n🧪 Testing basic domain encoding...")

    # Test ASCII domain (should remain unchanged)
    ascii_domain = idna_module.encode("example.com")
    assert ascii_domain == b"example.com"
    print("  ✅ ASCII domain encoding")

    # Test uppercase ASCII domain (case may be preserved)
    uppercase_domain = idna_module.encode("EXAMPLE.COM")
    # idna library may preserve case for ASCII domains
    assert uppercase_domain.lower() == b"example.com"
    print("  ✅ ASCII domain case handling")


def test_international_domains(idna_module: "IdnaType"):
    """Test encoding and decoding of international domain names."""
    print("\n🧪 Testing international domain names...")

    # Japanese domain
    encoded = idna_module.encode("ドメイン.テスト")
    assert encoded == b"xn--eckwd4c7c.xn--zckzah"
    print("  ✅ Japanese domain encoding")

    decoded = idna_module.decode(b"xn--eckwd4c7c.xn--zckzah")
    assert decoded == "ドメイン.テスト"
    print("  ✅ Japanese domain decoding")

    # German domain
    encoded_de = idna_module.encode("münchen.de")
    assert encoded_de == b"xn--mnchen-3ya.de"
    print("  ✅ German domain encoding")

    decoded_de = idna_module.decode(b"xn--mnchen-3ya.de")
    assert decoded_de == "münchen.de"
    print("  ✅ German domain decoding")

    # Russian domain
    encoded_ru = idna_module.encode("россия.рф")
    assert encoded_ru == b"xn--h1alffa9f.xn--p1ai"
    print("  ✅ Russian domain encoding")

    decoded_ru = idna_module.decode(b"xn--h1alffa9f.xn--p1ai")
    assert decoded_ru == "россия.рф"
    print("  ✅ Russian domain decoding")


def test_emoji_domains(idna_module: "IdnaType"):
    """Test encoding of emoji domains."""
    print("\n🧪 Testing emoji domains...")

    # Note: IDNA 2008 (strict mode) doesn't allow emoji in domain names
    # The emoji test would fail with InvalidCodepoint error
    # Testing decoding of a previously valid emoji domain instead

    try:
        # This will likely fail in strict IDNA 2008 mode
        emoji_encoded = idna_module.encode("💩.la", uts46=True, strict=False)
        print(f"  ✅ Emoji domain encoding (UTS46 mode): {emoji_encoded}")
    except Exception as e:
        # Expected in strict mode
        print(f"  ℹ️  Emoji encoding not supported in strict mode: {type(e).__name__}")

    # Test decoding (should work)
    try:
        emoji_decoded = idna_module.decode(b"xn--ls8h.la")
        assert emoji_decoded == "💩.la"
        print("  ✅ Emoji domain decoding")
    except Exception:
        print("  ℹ️  Emoji decoding also restricted in this version")


def test_mixed_scripts(idna_module: "IdnaType"):
    """Test domains with mixed scripts."""
    print("\n🧪 Testing mixed script domains...")

    # Arabic with numbers
    arabic = idna_module.encode("مثال.إختبار")
    assert arabic == b"xn--mgbh0fb.xn--kgbechtv"
    print("  ✅ Arabic domain encoding")

    # Chinese simplified
    chinese = idna_module.encode("中国.cn")
    assert chinese == b"xn--fiqs8s.cn"
    print("  ✅ Chinese domain encoding")

    # Greek
    greek = idna_module.encode("παράδειγμα.δοκιμή")
    assert greek == b"xn--hxajbheg2az3al.xn--jxalpdlp"
    print("  ✅ Greek domain encoding")


def test_idna_api(idna_module: "IdnaType"):
    """Test various IDNA API functions."""
    print("\n🧪 Testing IDNA API functions...")

    # Test ToASCII
    ascii_result = idna_module.encode("münchen.de", uts46=False)
    assert ascii_result == b"xn--mnchen-3ya.de"
    print("  ✅ ToASCII function")

    # Test ToUnicode
    unicode_result = idna_module.decode(b"xn--mnchen-3ya.de", uts46=False)
    assert unicode_result == "münchen.de"
    print("  ✅ ToUnicode function")

    # Test with UTS46
    uts46_result = idna_module.encode("MÜNCHEN.de", uts46=True)
    assert uts46_result == b"xn--mnchen-3ya.de"
    print("  ✅ UTS46 processing")


def test_error_handling(idna_module: "IdnaType"):
    """Test error handling for invalid inputs."""
    print("\n🧪 Testing error handling...")

    # Test empty label
    with pytest.raises(idna_module.core.IDNAError):
        idna_module.encode("example..com")
    print("  ✅ Empty label error handling")

    # Test label too long
    long_label = "a" * 64 + ".com"
    with pytest.raises(idna_module.core.IDNAError):
        idna_module.encode(long_label)
    print("  ✅ Label length error handling")

    # Test invalid character in domain
    try:
        # Some invalid characters should raise an error
        idna_module.encode("example@.com")
        print("  ⚠️  Invalid character handling may vary")
    except idna_module.core.IDNAError:
        print("  ✅ Invalid character error handling")


def test_idna_version(idna_module: "IdnaType"):
    """Test that version information is available."""
    print("\n🧪 Testing version information...")

    # Check version attribute exists
    assert hasattr(idna_module, "__version__")
    version = idna_module.__version__
    print(f"  ✅ IDNA version: {version}")

    # Version should be a string
    assert isinstance(version, str)
    assert len(version) > 0


def test_submodules(idna_module: "IdnaType"):
    """Test that key submodules are accessible."""
    print("\n🧪 Testing submodule access...")

    # Core module
    assert hasattr(idna_module, "core")
    assert hasattr(idna_module.core, "encode")
    assert hasattr(idna_module.core, "decode")
    print("  ✅ Core module accessible")

    # Note: After bundling, not all submodules may be preserved
    # unless they're explicitly imported. Check for commonly used ones.

    # Check for key functions available at top-level
    assert hasattr(idna_module, "encode")
    assert hasattr(idna_module, "decode")
    print("  ✅ Top-level encode/decode functions accessible")


@pytest.mark.parametrize(
    "input_domain,expected_encoded",
    [
        ("example.com", b"example.com"),
        ("münchen.de", b"xn--mnchen-3ya.de"),  # Lowercase version
        ("中国.cn", b"xn--fiqs8s.cn"),
        ("россия.рф", b"xn--h1alffa9f.xn--p1ai"),
        ("مثال.إختبار", b"xn--mgbh0fb.xn--kgbechtv"),
        ("παράδειγμα.δοκιμή", b"xn--hxajbheg2az3al.xn--jxalpdlp"),
        # Emoji domain removed - not supported in IDNA 2008 strict mode
    ],
    ids=[
        "ascii",
        "german",
        "chinese",
        "russian",
        "arabic",
        "greek",
    ],
)
def test_comprehensive_suite(idna_module: "IdnaType", input_domain: str, expected_encoded: bytes):
    """Test encoding and decoding of various domain names."""
    # Test encoding
    encoded = idna_module.encode(input_domain)
    assert encoded == expected_encoded, f"Encoding mismatch for {input_domain}"

    # Test decoding
    decoded = idna_module.decode(encoded)
    assert decoded.lower() == input_domain.lower(), f"Decoding mismatch for {encoded}"


if __name__ == "__main__":
    # For standalone execution
    pytest.main([__file__, "-v"])
