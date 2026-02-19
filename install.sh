#!/bin/bash

# ZeroAI Installation Script
# Automatically downloads and installs the latest release for your platform

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# GitHub repository
REPO="hushhenry/zeroai"
LATEST_RELEASE_URL="https://api.github.com/repos/${REPO}/releases/latest"

# Installation directory
INSTALL_DIR="${HOME}/.local/bin"
CONFIG_DIR="${HOME}/.zeroai"

# Binary name
BINARY_NAME="zeroai-proxy"

# Function to print colored output
print_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Function to detect OS and architecture
detect_platform() {
    local os
    local arch
    
    # Detect OS
    case "$(uname -s)" in
        Linux)
            os="linux"
            ;;
        Darwin)
            os="macos"
            ;;
        MINGW*|MSYS*|CYGWIN*)
            os="windows"
            ;;
        *)
            print_error "Unsupported OS: $(uname -s)"
            exit 1
            ;;
    esac
    
    # Detect architecture
    case "$(uname -m)" in
        x86_64|amd64)
            arch="x64"
            ;;
        aarch64|arm64)
            arch="arm64"
            ;;
        *)
            print_error "Unsupported architecture: $(uname -m)"
            exit 1
            ;;
    esac
    
    echo "${os}-${arch}"
}

# Function to get latest release version
get_latest_version() {
    print_info "Fetching latest release information..."
    
    if ! command -v curl &> /dev/null; then
        print_error "curl is required but not installed. Please install curl first."
        exit 1
    fi
    
    local response
    response=$(curl -s -L "${LATEST_RELEASE_URL}")
    
    if echo "$response" | grep -q "message"; then
        local error_msg=$(echo "$response" | grep -o '"message":"[^"]*"' | cut -d'"' -f4)
        print_error "Failed to fetch release info: $error_msg"
        exit 1
    fi
    
    local version=$(echo "$response" | grep -o '"tag_name":"[^"]*"' | cut -d'"' -f4)
    echo "$version"
}

# Function to download binary
download_binary() {
    local platform=$1
    local version=$2
    
    print_info "Downloading ZeroAI ${version} for ${platform}..."
    
    # Determine binary name based on platform
    local binary_file
    case "$platform" in
        linux-x64)
            binary_file="zeroai-proxy-linux-x64"
            ;;
        linux-arm64)
            binary_file="zeroai-proxy-linux-arm64"
            ;;
        macos-x64)
            binary_file="zeroai-proxy-macos-x64"
            ;;
        macos-arm64)
            binary_file="zeroai-proxy-macos-arm64"
            ;;
        windows-x64)
            binary_file="zeroai-proxy-windows-x64.exe"
            ;;
        windows-arm64)
            binary_file="zeroai-proxy-windows-arm64.exe"
            ;;
        *)
            print_error "Unsupported platform: ${platform}"
            exit 1
            ;;
    esac
    
    # Download URL
    local download_url="https://github.com/${REPO}/releases/download/${version}/${binary_file}"
    
    # Create temporary directory
    local temp_dir=$(mktemp -d)
    local temp_file="${temp_dir}/${binary_file}"
    
    # Download the binary
    if ! curl -L -o "${temp_file}" "${download_url}"; then
        print_error "Failed to download binary from ${download_url}"
        rm -rf "${temp_dir}"
        exit 1
    fi
    
    # Make executable (except on Windows)
    if [[ "$platform" != windows-* ]]; then
        chmod +x "${temp_file}"
    fi
    
    echo "${temp_file}"
}

# Function to install binary
install_binary() {
    local temp_file=$1
    local platform=$2
    
    print_info "Installing binary to ${INSTALL_DIR}..."
    
    # Create installation directory if it doesn't exist
    mkdir -p "${INSTALL_DIR}"
    
    # Determine binary name based on platform
    local binary_name="${BINARY_NAME}"
    if [[ "$platform" == windows-* ]]; then
        binary_name="${BINARY_NAME}.exe"
    fi
    
    # Move binary to installation directory
    mv "${temp_file}" "${INSTALL_DIR}/${binary_name}"
    
    # Make executable (except on Windows)
    if [[ "$platform" != windows-* ]]; then
        chmod +x "${INSTALL_DIR}/${binary_name}"
    fi
    
    # Add to PATH if not already there
    if [[ ":$PATH:" != *":${INSTALL_DIR}:"* ]]; then
        print_warning "Installation directory ${INSTALL_DIR} is not in your PATH."
        print_warning "Add the following line to your shell profile:"
        echo "  export PATH=\"\${PATH}:${INSTALL_DIR}\""
    fi
}

# Function to create config directory
create_config_dir() {
    print_info "Creating configuration directory..."
    mkdir -p "${CONFIG_DIR}"
    
    # Create initial config file if it doesn't exist
    local config_file="${CONFIG_DIR}/config.json"
    if [[ ! -f "${config_file}" ]]; then
        cat > "${config_file}" << EOF
{
  "providers": {}
}
EOF
        print_success "Created initial configuration file at ${config_file}"
    fi
}

# Function to show usage
show_usage() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  -h, --help          Show this help message"
    echo "  -v, --version       Show version to install (default: latest)"
    echo "  -d, --dir <DIR>     Installation directory (default: ${INSTALL_DIR})"
    echo "  -f, --force         Force installation even if already installed"
    echo ""
    echo "Examples:"
    echo "  $0                    # Install latest version"
    echo "  $0 --version v0.1.0   # Install specific version"
    echo "  $0 --dir /usr/local/bin  # Install to custom directory"
}

# Main installation function
main() {
    local target_version=""
    local target_dir="${INSTALL_DIR}"
    local force=false
    
    # Parse command line arguments
    while [[ $# -gt 0 ]]; do
        case $1 in
            -h|--help)
                show_usage
                exit 0
                ;;
            -v|--version)
                target_version="$2"
                shift 2
                ;;
            -d|--dir)
                target_dir="$2"
                shift 2
                ;;
            -f|--force)
                force=true
                shift
                ;;
            *)
                print_error "Unknown option: $1"
                show_usage
                exit 1
                ;;
        esac
    done
    
    # Update INSTALL_DIR
    INSTALL_DIR="${target_dir}"
    
    # Detect platform
    local platform
    platform=$(detect_platform)
    print_info "Detected platform: ${platform}"
    
    # Get latest version if not specified
    if [[ -z "${target_version}" ]]; then
        target_version=$(get_latest_version)
        print_info "Latest version: ${target_version}"
    else
        print_info "Target version: ${target_version}"
    fi
    
    # Check if already installed
    local binary_path="${INSTALL_DIR}/${BINARY_NAME}"
    if [[ "$platform" == windows-* ]]; then
        binary_path="${INSTALL_DIR}/${BINARY_NAME}.exe"
    fi
    
    if [[ -f "${binary_path}" ]] && [[ "${force}" != "true" ]]; then
        print_warning "ZeroAI is already installed at ${binary_path}"
        print_warning "Use --force to reinstall or update"
        exit 0
    fi
    
    # Download binary
    local temp_file
    temp_file=$(download_binary "${platform}" "${target_version}")
    
    # Install binary
    install_binary "${temp_file}" "${platform}"
    
    # Create config directory
    create_config_dir
    
    # Verify installation
    if [[ "$platform" != windows-* ]]; then
        if command -v "${BINARY_NAME}" &> /dev/null; then
            print_success "ZeroAI installed successfully!"
            print_info "Run '${BINARY_NAME} --help' to see available commands"
        else
            print_warning "Binary installed but not in PATH. Please add ${INSTALL_DIR} to your PATH."
        fi
    else
        print_success "ZeroAI installed successfully!"
        print_info "Run '${BINARY_NAME}.exe --help' to see available commands"
    fi
    
    # Show next steps
    echo ""
    print_info "Next steps:"
    echo "  1. Add ${INSTALL_DIR} to your PATH if not already done"
    echo "  2. Run '${BINARY_NAME} config' to configure providers"
    echo "  3. Run '${BINARY_NAME} serve' to start the proxy server"
}

# Run main function
main "$@"