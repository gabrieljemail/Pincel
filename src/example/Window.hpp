#pragma once

#include <iostream>
#include <cassert>
#include <cstdint>
#include <cstdio>
#include <string>
#include "glad.h"
#include <GLFW/glfw3.h>
#include <Pincel/Pincel.hpp>

class Window {
public:
	Window() = default;
	~Window() {
		if (texture) glDeleteTextures(1, &texture);
		if (program) glDeleteProgram(program);
		if (vao) glDeleteVertexArrays(1, &vao);
		if (window) glfwDestroyWindow(window);
		glfwTerminate();
	}

	void Init() {
		int err = glfwInit();
		if (!err) {
			std::cerr << "[ERROR] Failed to initialize GLFW. Error code: " << err << std::endl;
			glfwTerminate();
			std::exit(-1);
		}

		// Use OpenGL 4.6 Core.
		glfwWindowHint(GLFW_CONTEXT_VERSION_MAJOR, 4);
		glfwWindowHint(GLFW_CONTEXT_VERSION_MINOR, 6);
		glfwWindowHint(GLFW_OPENGL_PROFILE, GLFW_OPENGL_CORE_PROFILE);

		// Make our window.
		window = glfwCreateWindow(
			Window::WINDOW_SIZE[0],
			Window::WINDOW_SIZE[1],
			Window::WINDOW_TITLE,
			nullptr, nullptr
		);
		if (!window) {
			std::cerr << "[ERROR] Failed to create GLFW window." << std::endl;
			glfwTerminate();
			std::exit(-1);
		}
		glfwMakeContextCurrent(window);
		glfwSwapInterval(1); // vsync

		// Input that only comes as events. Pincel wants a snapshot per frame, so these just collect it until Render().
		glfwSetWindowUserPointer(window, this);
		glfwSetScrollCallback(window, OnScroll);
		glfwSetKeyCallback(window, OnKey);
		glfwSetCharCallback(window, OnChar);
		glfwSetCursorEnterCallback(window, OnCursorEnter);

		// Initialize the GLAD loader.
		if (!gladLoadGLLoader((GLADloadproc)glfwGetProcAddress)) {
			std::cerr << "[ERROR] Failed to initialize GLAD." << std::endl;
			glfwTerminate();
			std::exit(-1);
		}
		std::cout << "OpenGL " << glGetString(GL_VERSION) << " on " << glGetString(GL_RENDERER) << std::endl;

		CreatePipeline();
	}

	/// Lets the canvas's inputs copy to and paste from the operating system's clipboard, through GLFW.
	void ConnectClipboard(Pincel::Canvas* canvas) {
		assert(window != nullptr);
		GLFWwindow* handle = window;
		canvas->SetClipboard(
			[handle] { const char* text = glfwGetClipboardString(handle); return std::string(text ? text : ""); },
			[handle](const std::string& text) { glfwSetClipboardString(handle, text.c_str()); });
	}

	/// The height in pixels of the last frame rendered, which is the height of the canvas.
	int FramebufferHeight() const { return textureHeight; }

	bool ShouldClose() const {
		assert(window != nullptr);
		return glfwWindowShouldClose(window);
	}

	/// Renders one frame from Pincel's texture.
	void Render(Pincel::Canvas* canvas) {
		assert(window != nullptr);
		assert(Pincel::IsInitialized());

		glfwPollEvents();

		// The framebuffer size is in pixels, which differs from the window size on high-DPI displays.
		int width = 0, height = 0;
		glfwGetFramebufferSize(window, &width, &height);
		if (width <= 0 || height <= 0) return; // Minimized.
		if (width != textureWidth || height != textureHeight) ResizeTexture(width, height);

		// Give Pincel this frame's input, then rasterize and upload the pixels.
		double drawStart = glfwGetTime();
		canvas->SetSize(width, height);
		canvas->SetInput(CollectMouse(width, height), keyboard);
		Pincel::Texture* pincelTexture = canvas->Draw();
		double drawEnd = glfwGetTime();
		if (pincelTexture->version != uploadedVersion) { // Pincel only redraws when something changed, so this is often skipped.
			Upload(*pincelTexture);
			uploadedVersion = pincelTexture->version;
		}
		double uploadEnd = glfwGetTime();
		ReportTimings(drawEnd - drawStart, uploadEnd - drawEnd);

		// Pincel has seen the events, so start collecting the next frame's.
		scrollX = scrollY = 0.0f;
		keyboard.events.clear();
		keyboard.text.clear();

		// Then render.
		glViewport(0, 0, width, height);
		glClearColor(
			Window::CLEAR_COLOR[0],
			Window::CLEAR_COLOR[1],
			Window::CLEAR_COLOR[2],
			Window::CLEAR_COLOR[3]
		);
		glClear(GL_COLOR_BUFFER_BIT);

		glUseProgram(program);
		glBindTextureUnit(0, texture);
		glBindVertexArray(vao); // Core profile requires a VAO even though the triangle has no vertex data.
		glDrawArrays(GL_TRIANGLES, 0, 3);

		glfwSwapBuffers(window);
	}

private:
	static constexpr const float CLEAR_COLOR[4] = {0.95f, 0.95f, 0.95f, 1.00f};
	static constexpr const char* WINDOW_TITLE   = "Pincel Example";
	static constexpr const int   WINDOW_SIZE[2] = {854, 480};

	// One triangle that covers the whole screen, generated from gl_VertexID.
	// Row 0 of the CPU buffer is the top of the image, while OpenGL puts v=0 at the bottom of a texture, so v is flipped.
	static constexpr const char* VERTEX_SHADER = R"(#version 460 core
layout(location = 0) out vec2 uv;
void main() {
	vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
	uv = vec2(p.x, 1.0 - p.y);
	gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
)";

	static constexpr const char* FRAGMENT_SHADER = R"(#version 460 core
layout(location = 0) in vec2 uv;
layout(binding = 0) uniform sampler2D image;
layout(location = 0) out vec4 color;
void main() {
	color = texture(image, uv);
}
)";

	GLFWwindow* window = nullptr;

	// Timings, shown in the title bar twice a second. "Draw" is Pincel's layout and rasterizing on the CPU,
	// which is the part that matters; the frame rate itself is capped by vsync.
	double reportStart = 0.0;
	int reportFrames = 0;
	double reportDraw = 0.0;
	double reportUpload = 0.0;

	void ReportTimings(double drawSeconds, double uploadSeconds) {
		reportFrames++;
		reportDraw += drawSeconds;
		reportUpload += uploadSeconds;
		double now = glfwGetTime();
		if (now - reportStart < 0.5) return;

		char title[160];
		std::snprintf(title, sizeof(title), "%s - %.0f FPS - Pincel draw %.2f ms, upload %.2f ms",
			WINDOW_TITLE, reportFrames / (now - reportStart), reportDraw * 1000 / reportFrames, reportUpload * 1000 / reportFrames);
		glfwSetWindowTitle(window, title);
		reportStart = now;
		reportFrames = 0;
		reportDraw = reportUpload = 0.0;
	}

	// Input collected from GLFW's callbacks since the last frame.
	Pincel::KeyboardInput keyboard;
	float scrollX = 0.0f;
	float scrollY = 0.0f;
	bool cursorInside = false;

	static Window* From(GLFWwindow* w) { return static_cast<Window*>(glfwGetWindowUserPointer(w)); }

	static void OnScroll(GLFWwindow* w, double x, double y) {
		From(w)->scrollX += static_cast<float>(x);
		From(w)->scrollY += static_cast<float>(y);
	}

	static void OnChar(GLFWwindow* w, unsigned int codepoint) {
		From(w)->keyboard.text.push_back(static_cast<char32_t>(codepoint));
	}

	static void OnCursorEnter(GLFWwindow* w, int entered) { From(w)->cursorInside = entered != 0; }

	static void OnKey(GLFWwindow* w, int glfwKey, int, int glfwAction, int mods) {
		Window* self = From(w);
		self->keyboard.modifiers =
			(mods & GLFW_MOD_SHIFT ? Pincel::Modifier::Shift : 0) | (mods & GLFW_MOD_CONTROL ? Pincel::Modifier::Control : 0) |
			(mods & GLFW_MOD_ALT ? Pincel::Modifier::Alt : 0) | (mods & GLFW_MOD_SUPER ? Pincel::Modifier::Super : 0);

		Pincel::Key key = ToPincelKey(glfwKey);
		if (key == Pincel::Key::Unknown) return;
		Pincel::KeyAction action = glfwAction == GLFW_PRESS ? Pincel::KeyAction::Press
			: glfwAction == GLFW_REPEAT ? Pincel::KeyAction::Repeat : Pincel::KeyAction::Release;
		self->keyboard.events.push_back({key, action});
	}

	/// Translates GLFW's key codes to Pincel's.
	static Pincel::Key ToPincelKey(int key) {
		if (key >= GLFW_KEY_A && key <= GLFW_KEY_Z) return static_cast<Pincel::Key>(static_cast<int>(Pincel::Key::A) + (key - GLFW_KEY_A));
		switch (key) {
			case GLFW_KEY_BACKSPACE: return Pincel::Key::Backspace;
			case GLFW_KEY_DELETE: return Pincel::Key::Delete;
			case GLFW_KEY_ENTER: case GLFW_KEY_KP_ENTER: return Pincel::Key::Enter;
			case GLFW_KEY_TAB: return Pincel::Key::Tab;
			case GLFW_KEY_ESCAPE: return Pincel::Key::Escape;
			case GLFW_KEY_LEFT: return Pincel::Key::Left;
			case GLFW_KEY_RIGHT: return Pincel::Key::Right;
			case GLFW_KEY_UP: return Pincel::Key::Up;
			case GLFW_KEY_DOWN: return Pincel::Key::Down;
			case GLFW_KEY_HOME: return Pincel::Key::Home;
			case GLFW_KEY_END: return Pincel::Key::End;
			case GLFW_KEY_PAGE_UP: return Pincel::Key::PageUp;
			case GLFW_KEY_PAGE_DOWN: return Pincel::Key::PageDown;
			case GLFW_KEY_SPACE: return Pincel::Key::Space;
			default: return Pincel::Key::Unknown;
		}
	}

	/// The mouse as Pincel wants it: in framebuffer pixels (GLFW reports window units, which differ on high-DPI
	/// displays), and with no position at all when the cursor isn't over the window.
	Pincel::MouseInput CollectMouse(int framebufferWidth, int framebufferHeight) const {
		Pincel::MouseInput mouse; // The position starts as "not over the canvas".
		int windowWidth = 0, windowHeight = 0;
		glfwGetWindowSize(window, &windowWidth, &windowHeight);
		if (cursorInside && windowWidth > 0 && windowHeight > 0) {
			double x = 0, y = 0;
			glfwGetCursorPos(window, &x, &y);
			mouse.x = static_cast<float>(x * framebufferWidth / windowWidth);
			mouse.y = static_cast<float>(y * framebufferHeight / windowHeight);
		}
		if (glfwGetMouseButton(window, GLFW_MOUSE_BUTTON_LEFT) == GLFW_PRESS) mouse.buttons |= Pincel::MouseButton::Left;
		if (glfwGetMouseButton(window, GLFW_MOUSE_BUTTON_RIGHT) == GLFW_PRESS) mouse.buttons |= Pincel::MouseButton::Right;
		if (glfwGetMouseButton(window, GLFW_MOUSE_BUTTON_MIDDLE) == GLFW_PRESS) mouse.buttons |= Pincel::MouseButton::Middle;
		mouse.scrollX = scrollX;
		mouse.scrollY = scrollY;
		return mouse;
	}

	GLuint program = 0;
	GLuint vao = 0;
	GLuint texture = 0;
	int textureWidth = 0;
	int textureHeight = 0;
	std::uint64_t uploadedVersion = 0;

	static GLuint CompileShader(GLenum type, const char* source) {
		GLuint shader = glCreateShader(type);
		glShaderSource(shader, 1, &source, nullptr);
		glCompileShader(shader);

		GLint ok = 0;
		glGetShaderiv(shader, GL_COMPILE_STATUS, &ok);
		if (!ok) {
			char log[1024];
			glGetShaderInfoLog(shader, sizeof(log), nullptr, log);
			std::cerr << "[ERROR] Shader compilation failed:\n" << log << std::endl;
			std::exit(-1);
		}
		return shader;
	}

	void CreatePipeline() {
		GLuint vs = CompileShader(GL_VERTEX_SHADER, VERTEX_SHADER);
		GLuint fs = CompileShader(GL_FRAGMENT_SHADER, FRAGMENT_SHADER);
		program = glCreateProgram();
		glAttachShader(program, vs);
		glAttachShader(program, fs);
		glLinkProgram(program);
		glDeleteShader(vs);
		glDeleteShader(fs);

		GLint ok = 0;
		glGetProgramiv(program, GL_LINK_STATUS, &ok);
		if (!ok) {
			char log[1024];
			glGetProgramInfoLog(program, sizeof(log), nullptr, log);
			std::cerr << "[ERROR] Program link failed:\n" << log << std::endl;
			std::exit(-1);
		}

		glCreateVertexArrays(1, &vao);
	}

	/// (Re)creates the texture at the framebuffer size. Textures from glTextureStorage2D are immutable, so a resize makes a new one.
	void ResizeTexture(int width, int height) {
		if (texture) glDeleteTextures(1, &texture);
		glCreateTextures(GL_TEXTURE_2D, 1, &texture);
		glTextureStorage2D(texture, 1, GL_RGBA8, width, height);
		glTextureParameteri(texture, GL_TEXTURE_MIN_FILTER, GL_NEAREST); // 1:1 with the screen, so no filtering.
		glTextureParameteri(texture, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glTextureParameteri(texture, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
		glTextureParameteri(texture, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
		textureWidth = width;
		textureHeight = height;
		uploadedVersion = 0; // The new texture is empty, so whatever Pincel has has to be uploaded.
	}

	/// Copies Pincel's pixels into the GL texture. The texture size must match, which Render() ensures.
	void Upload(const Pincel::Texture& source) {
		assert(source.width == static_cast<std::uint32_t>(textureWidth) && source.height == static_cast<std::uint32_t>(textureHeight));
		glPixelStorei(GL_UNPACK_ALIGNMENT, 1); // Rows are tightly packed, whatever the width.
		glTextureSubImage2D(texture, 0, 0, 0, textureWidth, textureHeight, GL_RGBA, GL_UNSIGNED_BYTE, source.data);
	}
};
