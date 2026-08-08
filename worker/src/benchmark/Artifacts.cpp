#include "benchmark/Artifacts.h"

#include <algorithm>
#include <filesystem>
#include <fstream>
#include <vector>

#include <nlohmann/json.hpp>

#include "io/BrepCodec.h"

namespace onecad::benchmark {
namespace {

constexpr std::size_t kMaxArtifactBytes = 64U * 1024U * 1024U;

struct ArtifactBudget {
  std::size_t remaining = 0;
};

bool write_text(const std::filesystem::path &path, const std::string &text,
                ArtifactBudget &budget) {
  if (text.size() > budget.remaining)
    return false;
  std::ofstream output(path, std::ios::binary | std::ios::trunc);
  output.write(text.data(), static_cast<std::streamsize>(text.size()));
  if (!output.good())
    return false;
  budget.remaining -= text.size();
  return true;
}

bool write_bytes(const std::filesystem::path &path,
                 const std::vector<std::uint8_t> &bytes,
                 ArtifactBudget &budget) {
  if (bytes.size() > budget.remaining)
    return false;
  std::ofstream output(path, std::ios::binary | std::ios::trunc);
  output.write(reinterpret_cast<const char *>(bytes.data()),
               static_cast<std::streamsize>(bytes.size()));
  if (!output.good())
    return false;
  budget.remaining -= bytes.size();
  return true;
}

bool write_brep(const std::filesystem::path &path, const TopoDS_Shape &shape,
                ArtifactBudget &budget) {
  if (shape.IsNull())
    return false;
  std::vector<std::uint8_t> bytes;
  return onecad::io::write_brep_compound({shape}, bytes).empty() &&
         write_bytes(path, bytes, budget);
}

std::size_t artifact_limit(const Request &request) {
  const auto &limits = request.benchmark_case.limits;
  if (!limits.is_object() || !limits.contains("resources") ||
      !limits["resources"].contains("artifactBytes"))
    return kMaxArtifactBytes;
  return std::min<std::size_t>(kMaxArtifactBytes,
      limits["resources"]["artifactBytes"].get<std::size_t>());
}

} // namespace

nlohmann::json write_artifacts(const Request &request,
                               const GeneratedGeometry &geometry,
                               const AdapterResult &adapter) {
  nlohmann::json artifacts = {{"canonicalCase", nullptr}, {"request", nullptr},
                              {"stderr", nullptr}, {"inputShape", nullptr},
                              {"outputShape", nullptr}};
  if (request.artifact_dir.empty())
    return artifacts;
  try {
    const std::filesystem::path directory(request.artifact_dir);
    ArtifactBudget budget{artifact_limit(request)};
    std::filesystem::create_directories(directory);
    if (write_text(directory / "request.json", request.canonical.dump(), budget))
      artifacts["request"] = "request.json";
    if (write_text(directory / "case.json", request.benchmark_case.canonical.dump(), budget))
      artifacts["canonicalCase"] = "case.json";
    if (write_brep(directory / "input.brep", geometry.shape, budget))
      artifacts["inputShape"] = "input.brep";
    const TopoDS_Shape output = adapter.output.IsNull() ? adapter.partial_shape : adapter.output;
    if (write_brep(directory / "output.brep", output, budget))
      artifacts["outputShape"] = "output.brep";
    std::string stderr_text = adapter.message;
    if (!adapter.diagnostics.empty()) {
      if (!stderr_text.empty())
        stderr_text += '\n';
      stderr_text += adapter.diagnostics.dump();
    }
    if (!stderr_text.empty() &&
        write_text(directory / "stderr.txt", stderr_text, budget))
      artifacts["stderr"] = "stderr.txt";
  } catch (...) {
  }
  return artifacts;
}

} // namespace onecad::benchmark
