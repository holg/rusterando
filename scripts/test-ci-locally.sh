#!/usr/bin/env bash
# File: scripts/test-ci-locally.sh
# Local build & check script for rusterando

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${YELLOW}=== rusterando local checks ===${NC}\n"

# Step 1: cargo fmt
echo -e "${YELLOW}Step 1: cargo fmt check...${NC}"
if cargo fmt --all -- --check; then
    echo -e "${GREEN}✓ fmt passed${NC}\n"
else
    echo -e "${RED}✗ fmt failed${NC}"
    echo -e "${YELLOW}Run 'cargo fmt --all' to fix${NC}\n"
    exit 1
fi

# Step 2: clippy (SSR)
echo -e "${YELLOW}Step 2: clippy (SSR)...${NC}"
if cargo clippy --package rusterando-frontend --features ssr -- -D warnings; then
    echo -e "${GREEN}✓ clippy SSR passed${NC}\n"
else
    echo -e "${RED}✗ clippy SSR failed${NC}\n"
    exit 1
fi

# Step 3: check SSR build
echo -e "${YELLOW}Step 3: cargo check (SSR)...${NC}"
if cargo check --package rusterando-frontend --features ssr; then
    echo -e "${GREEN}✓ SSR build passed${NC}\n"
else
    echo -e "${RED}✗ SSR build failed${NC}\n"
    exit 1
fi

# Step 4: check WASM/hydrate build
echo -e "${YELLOW}Step 4: cargo check (WASM hydrate)...${NC}"
if cargo check --package rusterando-frontend --features hydrate --target wasm32-unknown-unknown; then
    echo -e "${GREEN}✓ WASM hydrate build passed${NC}\n"
else
    echo -e "${RED}✗ WASM hydrate build failed${NC}\n"
    exit 1
fi

# Step 5: check server build
echo -e "${YELLOW}Step 5: cargo check (server)...${NC}"
if cargo check --package rusterando-server --target aarch64-apple-darwin; then
    echo -e "${GREEN}✓ server build passed${NC}\n"
else
    echo -e "${RED}✗ server build failed${NC}\n"
    exit 1
fi

echo -e "\n${GREEN}=== All checks passed! ===${NC}"
