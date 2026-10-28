# Pincel
Pincel (Spanish for 'paintbrush') is a performant retained-mode UI rendering engine for video games and desktop applications. It is designed to be fast, easy to use, and scales with larger projects. It is written in C++ and Rust.

This project was created to replace Dear ImGui in Volcano Client, my Minecraft client also written in C++.

## How does it work?
First, there are three concepts you'll need to understand:
- Texture: An image stored on your GPU (Graphics Processing Unit) and shown on your screen in some way.
- Canvas: This has many meanings, but with Pincel it represents a set of elements that should be drawn to your screen.
- Element: In Pincel, an element is either a container (holds other elements) or a type of content such as text, images, etc.

There are two ways you'll interact with Pincel to draw your UIs:

### Elements
When you want to tell Pincel what you want to draw, you add a type of `Element` to the `Canvas`.
Pincel provides the following built-in Elements:
- `Text`: Draws text using style rules included in the Element, such as font, size, weight, color, etc.
- `Image`: Draws a texture from a provided image buffer, or reads a buffer from a provided file path.
- `Stack`: A container that organizes its children in order, either vertically or horizontally.
- `Button`: A container that can hold a click action, such as a button that sends you to another page.
- `Input`: An interactable element that allows users to type text or numbers in one or more lines.
- `Switch`: An interactable element that toggles between two boolean states, ON and OFF.
- `Select`: An interactable element that allows users to pick from multiple options and holds the selected index.

Elements are stored in heap memory as a tree, like this:
```text
+------------------------+
|         Button         |
| Action: () -> void     |
| Style: { ... }         |
+------------------------+
    |                |
Image(...)      Text("...")
```

You can define your own element type by defining a class that implements the virtual class `Element`, if using the C++ API:
```cpp
#include <Pincel/Element.hpp>
class MyElement : Pincel::Element { ... };
```

There are two ways to get stuff in your tree:
- **Programatically:** Use standard C++ classes and functions to make your own tree, such as using the `Pincel::Text("...", Pincel::Style { ... })` constructor. Because the C++ API eventually translates to Rust, you can also use Rust and link against the crate.
- **Declaratively:** Call `Pincel::Canvas::LoadFrom("...")` to load a tree from an XML file path.

If using the XML approach, here is an example:
```xml
<!-- Only text properties will be inherited to children, so the text below is also a dark theme, unless a child sets a value to exactly "inherit". -->
<Canvas background="#262626" foreground="#f7f7f7">
	<!-- Defines the base style properties for each element. -->
	<Theme>
		<!-- These tags don't actually make an element, they are base styles because they are inside a <Theme> tag. That's also why they are self-closing here. -->
		<!-- Also, properties that have a colon after them are objects, and after the colon is the key. -->
		<!-- Padding and margin have these keys, where right takes precedence over left: { u16 base, u16 x, u16 y, /* u16 top, right, bottom, left */ } -->
		<!-- Lastly, the hover key for colors is implemented independently by the element, which the built-in Button does implement. -->
		<Button background="#0060cb" background:hover="#2282ed" foreground="#f7f7f7" margin:top="8" alignItems:x="center" alignItems:y="center" padding:base="8" padding:x="12" cornerRadius="8" />
	</Theme>
	<Stack orientation="vertical">
		<Text fontSize="24" fontWeight="600" margin:bottom="8">Hello, World!</Text>
		<Text foreground="#a9a9a9">This page was created with XML and Pincel.</Text>
		<!-- Accessed with Canvas::GetFromID("...") -->
		<Button id="myButton" classes="button">
			<Text>Surprise Button</Text>
		</Button>
	</Stack>
</Canvas>
```

### Textures
Pincel is not an engine that does the whole stack for you. Because Pincel is designed to be embedded, your application needs to handle the graphics backend, while Pincel provides the rasterized texture for you to render.
For the same reason, you also need to provide the input data Pincel uses to handle interactions.

Here is an example that describes what you need to do each frame:
```cpp
#include <Pincel/Pincel.hpp>

using namespace Pincel;

Canvas* canvas = nullptr;
String pagePath = "pages/index.xml";

// You call this in another file.
void MyInit() {
	Pincel::Init(); // You can just use Init(), but this is clearer.
	Pincel::SetFlag("useAlpha", false); // This example is a desktop app, so disable alpha.
	canvas = GetCanvas();
	canvas->LoadFrom(pagePath); // The example above.
}

// Called each frame by your graphics handler.
void MyDrawMethod() {
	// Provide inputs.
	MouseInput mouse{};
	KeyboardInput keyboard{};
	// ...call your own input system methods...
	
	// Rasterize a texture from Pincel.
	Texture* texture = canvas->Draw();
	// ...upload the `texture->data` to the GPU...
}
```

## Using Pincel
If you are convinced by the stuff above, you'll be happy to know that Pincel is tiny!
To use it, you have two options:
1. **Linkng:** Add pincel.dll (or pincel.so) to your build system's link targets.
2. **Bundling:** Place Pincel's header files somewhere in your code and include them in your build system.

It is recommended to use the linking approach.

If you need help getting started, just clone this repo, which uses the bundling approach and creates an example desktop application with GLFW, OpenGL 4.6, and Pincel.
