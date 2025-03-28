import React, { useState, useRef, useEffect } from "react";
import { usePhotosStore, Photo } from "../../stores/PhotosStore";
import { Image, Plus, Trash2, Pencil, X, Save, Upload } from "lucide-react";

/**
 * Component props for the PhotosApp component
 */
export interface PhotosAppProps {
  /**
   * Optional title for the photos app
   */
  title?: string;
}

/**
 * A photo gallery application that allows uploading, viewing, and deleting photos
 * using keepsync for real-time synchronization across clients
 */
const PhotosApp: React.FC<PhotosAppProps> = ({ title = "My Photos" }) => {
  const { photos, addPhoto, updatePhoto, deletePhoto, getPhotoFile } = usePhotosStore();
  const [selectedPhotoId, setSelectedPhotoId] = useState<string | null>(null);
  const [photoUrls, setPhotoUrls] = useState<Record<string, string>>({});
  const [isUploading, setIsUploading] = useState<boolean>(false);
  const [isEditing, setIsEditing] = useState<boolean>(false);
  const [currentTitle, setCurrentTitle] = useState<string>("");
  const [currentDescription, setCurrentDescription] = useState<string>("");
  const [error, setError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Get the selected photo
  const selectedPhoto = selectedPhotoId
    ? photos.find(photo => photo.id === selectedPhotoId)
    : null;

  // Load photo URLs for all photos
  useEffect(() => {
    const loadPhotoUrls = async () => {
      const urls: Record<string, string> = {};
      
      for (const photo of photos) {
        if (!photoUrls[photo.fileHash]) {
          try {
            const blob = await getPhotoFile(photo.fileHash);
            if (blob) {
              urls[photo.fileHash] = URL.createObjectURL(blob);
            }
          } catch (error) {
            console.error(`Error loading photo ${photo.id}:`, error);
          }
        }
      }
      
      setPhotoUrls(prev => ({ ...prev, ...urls }));
    };
    
    loadPhotoUrls();
    
    // Cleanup function to revoke object URLs
    return () => {
      Object.values(photoUrls).forEach(url => URL.revokeObjectURL(url));
    };
  }, [photos, getPhotoFile, photoUrls]);

  // Handle file upload
  const handleFileUpload = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const files = event.target.files;
    if (!files || files.length === 0) return;
    
    const file = files[0];
    
    // Only allow image files
    if (!file.type.startsWith("image/")) {
      setError("Only image files are supported");
      return;
    }
    
    // Reset file input
    if (fileInputRef.current) {
      fileInputRef.current.value = "";
    }
    
    setIsUploading(true);
    setError(null);
    
    try {
      // Default title is the filename without extension
      const defaultTitle = file.name.split(".")[0] || "Untitled Photo";
      
      await addPhoto(defaultTitle, "", file);
      setIsUploading(false);
    } catch (error) {
      console.error("Error uploading photo:", error);
      setError("Failed to upload photo. Please try again.");
      setIsUploading(false);
    }
  };

  // Select a photo for viewing/editing
  const handleSelectPhoto = (photo: Photo) => {
    setSelectedPhotoId(photo.id);
    setCurrentTitle(photo.title);
    setCurrentDescription(photo.description);
    setIsEditing(false);
  };

  // Update the selected photo
  const handleSavePhoto = () => {
    if (selectedPhotoId && currentTitle.trim()) {
      updatePhoto(selectedPhotoId, currentTitle, currentDescription);
      setIsEditing(false);
    }
  };

  // Delete the selected photo
  const handleDeletePhoto = async () => {
    if (selectedPhotoId) {
      try {
        await deletePhoto(selectedPhotoId);
        setSelectedPhotoId(null);
        setCurrentTitle("");
        setCurrentDescription("");
        setIsEditing(false);
      } catch (error) {
        console.error("Error deleting photo:", error);
        setError("Failed to delete photo. Please try again.");
      }
    }
  };

  // Format the date to a readable string
  const formatDate = (timestamp: number) => {
    return new Date(timestamp).toLocaleString();
  };

  return (
    <div className="flex h-full rounded-lg border border-gray-200 overflow-hidden">
      {/* Photos sidebar */}
      <div className="w-1/3 border-r border-gray-200 bg-gray-50 flex flex-col">
        <div className="p-4 border-b border-gray-200 bg-white flex justify-between items-center">
          <h2 className="text-lg font-semibold">{title}</h2>
          <input
            type="file"
            ref={fileInputRef}
            onChange={handleFileUpload}
            accept="image/*"
            className="hidden"
            id="photo-upload"
          />
          <label
            htmlFor="photo-upload"
            className={`p-2 rounded-full hover:bg-gray-100 cursor-pointer ${
              isUploading ? "opacity-50 cursor-not-allowed" : ""
            }`}
            aria-label="Upload photo"
          >
            <Upload size={20} />
          </label>
        </div>
        
        {error && (
          <div className="m-4 p-2 bg-red-50 text-red-600 rounded-md text-sm">
            {error}
            <button 
              onClick={() => setError(null)}
              className="ml-2 text-red-800 hover:text-red-900"
              aria-label="Clear error"
            >
              <X size={16} />
            </button>
          </div>
        )}
        
        <div className="flex-1 overflow-y-auto">
          {photos.length === 0 ? (
            <div className="p-4 text-gray-500 text-center">
              No photos yet. Click the upload button to add some.
            </div>
          ) : (
            <div className="grid grid-cols-2 gap-2 p-2">
              {photos.map((photo) => (
                <button
                  key={photo.id}
                  onClick={() => handleSelectPhoto(photo)}
                  className={`relative rounded-md overflow-hidden hover:opacity-90 transition-opacity ${
                    selectedPhotoId === photo.id ? "ring-2 ring-blue-500" : ""
                  }`}
                >
                  {photoUrls[photo.fileHash] ? (
                    <img
                      src={photoUrls[photo.fileHash]}
                      alt={photo.title}
                      className="w-full h-24 object-cover"
                    />
                  ) : (
                    <div className="w-full h-24 bg-gray-200 flex items-center justify-center">
                      <Image size={24} className="text-gray-400" />
                    </div>
                  )}
                  <div className="absolute bottom-0 left-0 right-0 bg-black bg-opacity-50 text-white p-1 text-xs truncate">
                    {photo.title}
                  </div>
                </button>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* Photo viewer/editor */}
      <div className="w-2/3 flex flex-col">
        {selectedPhoto ? (
          <>
            <div className="p-4 border-b border-gray-200 flex justify-between items-center">
              <div>
                {isEditing ? (
                  <input
                    type="text"
                    value={currentTitle}
                    onChange={(e) => setCurrentTitle(e.target.value)}
                    className="border-b border-gray-300 bg-transparent focus:outline-none focus:border-blue-500 px-1"
                    placeholder="Photo title"
                  />
                ) : (
                  <div className="font-medium">{selectedPhoto.title}</div>
                )}
                <div className="text-sm text-gray-500">
                  Added: {formatDate(selectedPhoto.createdAt)}
                </div>
              </div>
              <div className="flex gap-2">
                {isEditing ? (
                  <button
                    onClick={handleSavePhoto}
                    className="p-2 rounded-full text-blue-600 hover:bg-blue-50"
                    aria-label="Save photo details"
                  >
                    <Save size={20} />
                  </button>
                ) : (
                  <button
                    onClick={() => setIsEditing(true)}
                    className="p-2 rounded-full text-blue-600 hover:bg-blue-50"
                    aria-label="Edit photo details"
                  >
                    <Pencil size={20} />
                  </button>
                )}
                <button
                  onClick={handleDeletePhoto}
                  className="p-2 rounded-full text-red-600 hover:bg-red-50"
                  aria-label="Delete photo"
                >
                  <Trash2 size={20} />
                </button>
              </div>
            </div>
            <div className="flex-1 p-4 flex flex-col">
              <div className="mb-4 flex-grow overflow-hidden flex items-center justify-center bg-gray-100 rounded-md">
                {photoUrls[selectedPhoto.fileHash] ? (
                  <img
                    src={photoUrls[selectedPhoto.fileHash]}
                    alt={selectedPhoto.title}
                    className="max-w-full max-h-full object-contain"
                  />
                ) : (
                  <div className="flex items-center justify-center h-full">
                    <Image size={48} className="text-gray-400" />
                    <p className="ml-2 text-gray-500">Loading image...</p>
                  </div>
                )}
              </div>
              {isEditing ? (
                <textarea
                  value={currentDescription}
                  onChange={(e) => setCurrentDescription(e.target.value)}
                  className="w-full p-2 border border-gray-200 rounded-md resize-none focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                  placeholder="Add a description..."
                  rows={3}
                />
              ) : (
                <div className="mt-2 text-gray-700">
                  {selectedPhoto.description || "No description"}
                </div>
              )}
            </div>
          </>
        ) : (
          <div className="flex-1 flex flex-col items-center justify-center text-gray-500">
            <Image size={48} className="text-gray-300 mb-4" />
            <p>Select a photo or upload a new one to get started.</p>
            <input
              type="file"
              onChange={handleFileUpload}
              accept="image/*"
              className="hidden"
              id="photo-upload-center"
            />
            <label
              htmlFor="photo-upload-center"
              className="mt-4 px-4 py-2 bg-blue-500 text-white rounded-md hover:bg-blue-600 cursor-pointer"
            >
              Upload Photo
            </label>
          </div>
        )}
      </div>
    </div>
  );
};

export default PhotosApp;