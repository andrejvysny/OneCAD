#include <iostream>
#include <iterator>
#include <string>

#include <Standard_Failure.hxx>
#include <nlohmann/json.hpp>

#include "benchmark/CaseParser.h"
#include "benchmark/Execution.h"

namespace {

constexpr std::size_t kMaxRequestBytes = 1024U * 1024U;

bool read_request(std::string &text) {
  char buffer[8192];
  while (std::cin.good()) {
    std::cin.read(buffer, sizeof(buffer));
    const std::streamsize count = std::cin.gcount();
    if (count > 0)
      text.append(buffer, static_cast<std::size_t>(count));
    if (text.size() > kMaxRequestBytes)
      return false;
  }
  return !text.empty();
}

void schema_error(const std::string &message) {
  std::cerr << "kernelbench request error: " << message << '\n';
}

} // namespace

int main() {
  std::string text;
  if (!read_request(text)) {
    schema_error(text.empty() ? "empty input" : "input exceeds 1 MiB");
    return 2;
  }
  onecad::benchmark::Request request;
  try {
    const nlohmann::json input = nlohmann::json::parse(text);
    std::string error;
    if (!onecad::benchmark::parse_request(input, request, error)) {
      schema_error(error);
      return 2;
    }
    std::cout << onecad::benchmark::execute_request(request).dump() << '\n';
    return 0;
  } catch (const nlohmann::json::exception &error) {
    schema_error(error.what());
    return 2;
  } catch (const Standard_Failure &failure) {
    std::cout << onecad::benchmark::exception_result(
                     &request, "exception",
                     failure.what())
                     .dump()
              << '\n';
  } catch (const std::exception &error) {
    std::cout << onecad::benchmark::exception_result(
                     &request, "exception", error.what())
                     .dump()
              << '\n';
  } catch (...) {
    std::cout << onecad::benchmark::exception_result(
                     &request, "exception", "unknown runner exception")
                     .dump()
              << '\n';
  }
  return 0;
}
