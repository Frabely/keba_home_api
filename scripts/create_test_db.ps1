param(
  [string]$Path = ".\data\keba_test.db",
  [switch]$Force
)

$args = @("--path", $Path)
if ($Force) {
  $args += "--force"
}

cargo run --bin create_test_db -- @args
